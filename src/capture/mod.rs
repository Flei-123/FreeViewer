use image::{ImageBuffer, RgbaImage};

pub mod screen;
pub mod audio;

pub use screen::{ScreenCapture as ScreenCaptureImpl, ScreenInfo};
pub use audio::AudioCapture;

/// Main capture module for screen and audio recording
pub struct CaptureManager {
    screen_capture: ScreenCaptureImpl,
    audio_capture: Option<AudioCapture>,
    is_capturing: bool,
}

impl CaptureManager {
    pub fn new() -> Self {
        Self {
            screen_capture: ScreenCaptureImpl::new(),
            audio_capture: None,
            is_capturing: false,
        }
    }
    
    /// Start capturing screen and optionally audio
    pub async fn start_capture(&mut self, include_audio: bool) -> Result<(), CaptureError> {
        if self.is_capturing {
            return Err(CaptureError::AlreadyCapturing);
        }
        
        // Start screen capture
        self.screen_capture.start().await?;
        
        // Start audio capture if requested
        if include_audio {
            let mut audio_capture = AudioCapture::new();
            audio_capture.start().await?;
            self.audio_capture = Some(audio_capture);
        }
        
        self.is_capturing = true;
        tracing::info!("Capture started (audio: {})", include_audio);
        
        Ok(())
    }
    
    /// Stop capturing
    pub async fn stop_capture(&mut self) -> Result<(), CaptureError> {
        if !self.is_capturing {
            return Err(CaptureError::NotCapturing);
        }
        
        // Stop screen capture
        self.screen_capture.stop().await?;
        
        // Stop audio capture if running
        if let Some(mut audio_capture) = self.audio_capture.take() {
            audio_capture.stop().await?;
        }
        
        self.is_capturing = false;
        tracing::info!("Capture stopped");
        
        Ok(())
    }
    
    /// Capture a single screen frame
    pub async fn capture_frame(&mut self) -> Result<CaptureFrame, CaptureError> {
        if !self.is_capturing {
            return Err(CaptureError::NotCapturing);
        }
        
        let screen_data = self.screen_capture.capture_frame().await?;
        
        Ok(CaptureFrame {
            screen: screen_data,
            audio: None, // TODO: Capture audio frame
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis() as u64,
        })
    }
    
    /// Get screen resolution
    pub fn get_screen_resolution(&self) -> (u32, u32) {
        self.screen_capture.get_resolution()
    }
    
    /// Set screen capture quality
    pub fn set_quality(&mut self, quality: CaptureQuality) {
        self.screen_capture.set_quality(quality);
    }
    
    /// Check if currently capturing
    pub fn is_capturing(&self) -> bool {
        self.is_capturing
    }
}

/// A captured frame containing screen and optionally audio data
#[derive(Debug, Clone)]
pub struct CaptureFrame {
    pub screen: ScreenFrame,
    pub audio: Option<AudioFrame>,
    pub timestamp: u64,
}

/// Screen frame data
#[derive(Debug, Clone)]
pub struct ScreenFrame {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub format: ScreenFormat,
}

/// Audio frame data
#[derive(Debug, Clone)]
pub struct AudioFrame {
    pub data: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Screen capture format
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScreenFormat {
    Rgba8,
    Rgb8,
    Bgra8,
    Bgr8,
}

/// Capture quality settings
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CaptureQuality {
    Low,    // Fast, low quality
    Medium, // Balanced
    High,   // Slow, high quality
    Lossless, // Very slow, perfect quality
}

impl CaptureQuality {
    pub fn compression_level(&self) -> u8 {
        match self {
            CaptureQuality::Low => 1,
            CaptureQuality::Medium => 6,
            CaptureQuality::High => 9,
            CaptureQuality::Lossless => 0, // No compression
        }
    }
    
    pub fn frame_rate(&self) -> u32 {
        match self {
            CaptureQuality::Low => 15,
            CaptureQuality::Medium => 30,
            CaptureQuality::High => 60,
            CaptureQuality::Lossless => 30,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("Already capturing")]
    AlreadyCapturing,
    
    #[error("Not currently capturing")]
    NotCapturing,
    
    #[error("Screen capture failed: {0}")]
    ScreenCaptureFailed(String),
    
    #[error("Audio capture failed: {0}")]
    AudioCaptureFailed(String),
    
    #[error("Compression failed: {0}")]
    CompressionFailed(String),
    
    #[error("Invalid format")]
    InvalidFormat,
    
    #[error("System error: {0}")]
    SystemError(String),
    
    #[error("Screen access error: {0}")]
    ScreenAccessError(String),
    
    #[error("No screens found")]
    NoScreensFound,
    
    #[error("Capture failure: {0}")]
    CaptureFailure(String),
    
    #[error("Encoding error: {0}")]
    EncodingError(String),
    
    #[error("Task error: {0}")]
    TaskError(String),
    
    #[error("Invalid parameters")]
    InvalidParameters,
}
