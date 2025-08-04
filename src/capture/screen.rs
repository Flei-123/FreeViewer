use screenshots::Screen;
use image::{GenericImageView, ImageOutputFormat, DynamicImage, ImageBuffer, Rgba};
use std::sync::Arc;
use tokio::time::{interval, Duration, Instant};
use tokio::sync::{broadcast, RwLock};
use crate::capture::{CaptureError, ScreenFrame, ScreenFormat, CaptureQuality};

/// Real screen capture manager with full functionality
pub struct ScreenCapture {
    resolution: (u32, u32),
    is_active: bool,
    quality: CaptureQuality,
    frame_rate: u32,
    capture_sender: Option<broadcast::Sender<ScreenFrame>>,
    is_capturing: Arc<std::sync::atomic::AtomicBool>,
    
    // Real capture state
    screens: Vec<Screen>,
    current_screen_idx: usize,
    last_capture_time: Arc<RwLock<Option<Instant>>>,
    frame_cache: Arc<RwLock<Option<ScreenFrame>>>,
    capture_stats: Arc<RwLock<CaptureStats>>,
}

#[derive(Debug, Clone)]
pub struct CaptureStats {
    pub total_frames: u64,
    pub fps: f32,
    pub avg_capture_time_ms: f32,
    pub last_error: Option<String>,
}

impl ScreenCapture {
    pub fn new() -> Self {
        let screens = Screen::all().unwrap_or_default();
        let resolution = if let Some(screen) = screens.first() {
            (screen.display_info.width, screen.display_info.height)
        } else {
            (1920, 1080)
        };

        Self {
            resolution,
            is_active: false,
            quality: CaptureQuality::Medium,
            frame_rate: 30,
            capture_sender: None,
            is_capturing: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            
            screens,
            current_screen_idx: 0,
            last_capture_time: Arc::new(RwLock::new(None)),
            frame_cache: Arc::new(RwLock::new(None)),
            capture_stats: Arc::new(RwLock::new(CaptureStats {
                total_frames: 0,
                fps: 0.0,
                avg_capture_time_ms: 0.0,
                last_error: None,
            })),
        }
    }
    
    /// Get capture statistics
    pub async fn get_stats(&self) -> CaptureStats {
        self.capture_stats.read().await.clone()
    }
    
    /// Switch to a different screen
    pub fn set_screen(&mut self, screen_idx: usize) -> Result<(), CaptureError> {
        if screen_idx >= self.screens.len() {
            return Err(CaptureError::InvalidParameters);
        }
        self.current_screen_idx = screen_idx;
        if let Some(screen) = self.screens.get(screen_idx) {
            self.resolution = (screen.display_info.width, screen.display_info.height);
        }
        Ok(())
    }
    
    /// Set capture quality
    pub fn set_quality(&mut self, quality: CaptureQuality) {
        self.quality = quality;
    }
    
    /// Set target frame rate
    pub fn set_frame_rate(&mut self, fps: u32) {
        self.frame_rate = fps.clamp(1, 120);
    }
    
    /// Start screen capture
    pub async fn start(&mut self) -> Result<(), CaptureError> {
        if self.is_active {
            return Err(CaptureError::AlreadyCapturing);
        }
        
        self.is_active = true;
        tracing::info!("Screen capture started");
        Ok(())
    }
    
    /// Stop screen capture
    pub async fn stop(&mut self) -> Result<(), CaptureError> {
        if !self.is_active {
            return Err(CaptureError::NotCapturing);
        }
        
        self.is_active = false;
        tracing::info!("Screen capture stopped");
        Ok(())
    }
    
    /// Capture a single frame with real implementation
    pub async fn capture_frame(&mut self) -> Result<ScreenFrame, CaptureError> {
        if !self.is_active {
            return Err(CaptureError::NotCapturing);
        }
        
        let start_time = Instant::now();
        let screen_idx = self.current_screen_idx;
        let quality = self.quality.clone();
        let last_capture_time = Arc::clone(&self.last_capture_time);
        let capture_stats = Arc::clone(&self.capture_stats);
        
        let frame = tokio::task::spawn_blocking(move || {
            let screens = Screen::all()
                .map_err(|e| CaptureError::ScreenAccessError(e.to_string()))?;
            
            if screens.is_empty() {
                return Err(CaptureError::NoScreensFound);
            }
            
            let screen = screens.get(screen_idx)
                .ok_or_else(|| CaptureError::InvalidParameters)?;
            
            // Actual screen capture
            let image = screen.capture()
                .map_err(|e| CaptureError::CaptureFailure(e.to_string()))?;
            
            // Process based on quality settings
            let processed_data = match quality {
                CaptureQuality::Lossless => {
                    // Raw RGBA data for lossless
                    image.as_raw().to_vec()
                },
                _ => {
                    // Convert to DynamicImage for compression
                    let dynamic_image = DynamicImage::ImageRgba8(image.clone());
                    Self::compress_image(&dynamic_image, &quality)?
                }
            };
            
            Ok(ScreenFrame {
                data: processed_data,
                width: image.width(),
                height: image.height(),
                format: match quality {
                    CaptureQuality::Lossless => ScreenFormat::Rgba8,
                    _ => ScreenFormat::Rgba8, // Can be changed to compressed format
                },
            })
        }).await
        .map_err(|e| CaptureError::TaskError(e.to_string()))??;
        
        // Update stats
        let capture_time = start_time.elapsed();
        self.update_stats(capture_time).await;
        
        // Cache the frame
        *self.frame_cache.write().await = Some(frame.clone());
        
        Ok(frame)
    }
    
    /// Compress image based on quality setting
    fn compress_image(image: &DynamicImage, quality: &CaptureQuality) -> Result<Vec<u8>, CaptureError> {
        let jpeg_quality = match quality {
            CaptureQuality::Low => 40,
            CaptureQuality::Medium => 70,
            CaptureQuality::High => 90,
            CaptureQuality::Lossless => 100,
        };
        
        let mut buffer = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buffer);
        
        image.write_to(&mut cursor, ImageOutputFormat::Jpeg(jpeg_quality))
            .map_err(|e| CaptureError::EncodingError(e.to_string()))?;
        
        Ok(buffer)
    }
    
    /// Update capture statistics
    async fn update_stats(&self, capture_time: Duration) {
        let mut stats = self.capture_stats.write().await;
        stats.total_frames += 1;
        
        let capture_time_ms = capture_time.as_millis() as f32;
        if stats.total_frames == 1 {
            stats.avg_capture_time_ms = capture_time_ms;
        } else {
            // Exponential moving average
            stats.avg_capture_time_ms = stats.avg_capture_time_ms * 0.9 + capture_time_ms * 0.1;
        }
        
        // Calculate FPS
        if capture_time_ms > 0.0 {
            stats.fps = 1000.0 / stats.avg_capture_time_ms;
        }
        
        *self.last_capture_time.write().await = Some(Instant::now());
    }
    
    /// Get current resolution
    pub fn get_resolution(&self) -> (u32, u32) {
        self.resolution
    }
    
    /// Start continuous capture
    pub async fn start_capture(&mut self) -> Result<broadcast::Receiver<ScreenFrame>, CaptureError> {
        if self.is_capturing.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(CaptureError::AlreadyCapturing);
        }
        
        let (sender, receiver) = broadcast::channel(100);
        self.capture_sender = Some(sender.clone());
        
        let is_capturing = Arc::clone(&self.is_capturing);
        let quality = self.quality.clone();
        let frame_rate = self.frame_rate;
        
        // Start capture task
        tokio::spawn(async move {
            is_capturing.store(true, std::sync::atomic::Ordering::Relaxed);
            
            let mut interval = interval(Duration::from_millis(1000 / frame_rate as u64));
            
            while is_capturing.load(std::sync::atomic::Ordering::Relaxed) {
                interval.tick().await;
                
                match Self::capture_screen_internal(&quality).await {
                    Ok(frame) => {
                        if sender.send(frame).is_err() {
                            // No receivers left, stop capturing
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Screen capture error: {}", e);
                        // Continue trying to capture
                    }
                }
            }
            
            is_capturing.store(false, std::sync::atomic::Ordering::Relaxed);
        });
        
        Ok(receiver)
    }
    
    /// Stop continuous capture
    pub fn stop_capture(&mut self) {
        self.is_capturing.store(false, std::sync::atomic::Ordering::Relaxed);
        self.capture_sender = None;
    }
    
    /// Internal screen capture implementation
    async fn capture_screen_internal(quality: &CaptureQuality) -> Result<ScreenFrame, CaptureError> {
        let quality = quality.clone();
        tokio::task::spawn_blocking(move || {
            let screens = Screen::all()
                .map_err(|e| CaptureError::ScreenAccessError(e.to_string()))?;
            
            if screens.is_empty() {
                return Err(CaptureError::NoScreensFound);
            }
            
            // Capture primary screen
            let screen = &screens[0];
            let image = screen.capture()
                .map_err(|e| CaptureError::CaptureFailure(e.to_string()))?;
            
            // Convert based on quality setting
            let data = match quality {
                CaptureQuality::Lossless => {
                    // Use PNG for lossless
                    let mut png_data = Vec::new();
                    let mut cursor = std::io::Cursor::new(&mut png_data);
                    
                    image.write_to(&mut cursor, ImageOutputFormat::Png)
                        .map_err(|e| CaptureError::EncodingError(e.to_string()))?;
                    
                    png_data
                }
                _ => {
                    // Use JPEG for lossy compression
                    let jpeg_quality = match quality {
                        CaptureQuality::Low => 30,
                        CaptureQuality::Medium => 60,
                        CaptureQuality::High => 80,
                        CaptureQuality::Lossless => 100, // Fallback
                    };
                    
                    let mut jpeg_data = Vec::new();
                    let mut cursor = std::io::Cursor::new(&mut jpeg_data);
                    
                    image.write_to(&mut cursor, ImageOutputFormat::Jpeg(jpeg_quality))
                        .map_err(|e| CaptureError::EncodingError(e.to_string()))?;
                    
                    jpeg_data
                }
            };
            
            Ok(ScreenFrame {
                data,
                width: image.width(),
                height: image.height(),
                format: ScreenFormat::Rgba8,
            })
        }).await
        .map_err(|e| CaptureError::TaskError(e.to_string()))?
    }
    
    /// Check if currently capturing
    pub fn is_capturing(&self) -> bool {
        self.is_capturing.load(std::sync::atomic::Ordering::Relaxed)
    }
    
    /// Get available screens
    pub async fn get_screens() -> Result<Vec<ScreenInfo>, CaptureError> {
        tokio::task::spawn_blocking(|| {
            let screens = Screen::all()
                .map_err(|e| CaptureError::ScreenAccessError(e.to_string()))?;
            
            let screen_info: Vec<ScreenInfo> = screens
                .into_iter()
                .enumerate()
                .map(|(index, screen)| ScreenInfo {
                    id: index,
                    name: format!("Screen {}", index + 1),
                    width: screen.display_info.width,
                    height: screen.display_info.height,
                    x: screen.display_info.x,
                    y: screen.display_info.y,
                    is_primary: index == 0,
                })
                .collect();
            
            Ok(screen_info)
        }).await
        .map_err(|e| CaptureError::TaskError(e.to_string()))?
    }
}

#[derive(Debug, Clone)]
pub struct ScreenInfo {
    pub id: usize,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub x: i32,
    pub y: i32,
    pub is_primary: bool,
}

impl Default for ScreenCapture {
    fn default() -> Self {
        Self::new()
    }
}
