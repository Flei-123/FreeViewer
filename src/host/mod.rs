use std::sync::Arc;
use tokio::sync::Mutex;
use crate::protocol::{Message, ConnectionState};

pub mod screen_capture;
pub mod input_handler;
pub mod file_server;

pub use screen_capture::ScreenCapture;
pub use input_handler::InputHandler;
pub use file_server::FileServer;

/// Host service that allows incoming remote connections
pub struct FreeViewerHost {
    screen_capture: Arc<Mutex<ScreenCapture>>,
    input_handler: Arc<Mutex<InputHandler>>,
    file_server: Arc<Mutex<FileServer>>,
    is_running: Arc<Mutex<bool>>,
    partner_id: String,
}

impl FreeViewerHost {
    pub fn new(partner_id: String) -> Self {
        Self {
            screen_capture: Arc::new(Mutex::new(ScreenCapture::new())),
            input_handler: Arc::new(Mutex::new(InputHandler::new())),
            file_server: Arc::new(Mutex::new(FileServer::new())),
            is_running: Arc::new(Mutex::new(false)),
            partner_id,
        }
    }
    
    /// Start the host service
    pub async fn start(&self) -> Result<(), HostError> {
        let mut is_running = self.is_running.lock().await;
        if *is_running {
            return Err(HostError::AlreadyRunning);
        }
        
        *is_running = true;
        
        // Start screen capture
        let screen_capture = self.screen_capture.clone();
        tokio::spawn(async move {
            let mut capture = screen_capture.lock().await;
            capture.start_capture().await;
        });
        
        // Start input handler
        let input_handler = self.input_handler.clone();
        tokio::spawn(async move {
            let mut handler = input_handler.lock().await;
            handler.start().await;
        });
        
        // Start file server
        let file_server = self.file_server.clone();
        tokio::spawn(async move {
            let mut server = file_server.lock().await;
            server.start().await;
        });
        
        tracing::info!("FreeViewer host started with ID: {}", self.partner_id);
        Ok(())
    }
    
    /// Stop the host service
    pub async fn stop(&self) -> Result<(), HostError> {
        let mut is_running = self.is_running.lock().await;
        if !*is_running {
            return Err(HostError::NotRunning);
        }
        
        *is_running = false;
        
        // Stop all services
        let mut screen_capture = self.screen_capture.lock().await;
        screen_capture.stop_capture().await;
        
        let mut input_handler = self.input_handler.lock().await;
        input_handler.stop().await;
        
        let mut file_server = self.file_server.lock().await;
        file_server.stop().await;
        
        tracing::info!("FreeViewer host stopped");
        Ok(())
    }
    
    /// Handle incoming message from client
    pub async fn handle_message(&self, message: Message) -> Result<Option<Message>, HostError> {
        match message {
            Message::MouseMove { x, y } => {
                let mut input_handler = self.input_handler.lock().await;
                input_handler.move_mouse(x, y).await?;
                Ok(None)
            }
            
            Message::MouseClick { x, y, button, pressed } => {
                let mut input_handler = self.input_handler.lock().await;
                input_handler.click_mouse(x, y, button, pressed).await?;
                Ok(None)
            }
            
            Message::KeyPress { key, pressed, modifiers } => {
                let mut input_handler = self.input_handler.lock().await;
                input_handler.press_key(key, pressed, modifiers).await?;
                Ok(None)
            }
            
            Message::FileListRequest { path } => {
                let mut file_server = self.file_server.lock().await;
                let files = file_server.list_files(path).await?;
                Ok(Some(Message::FileListResponse { files }))
            }
            
            Message::ScreenResolution { width, height } => {
                let mut screen_capture = self.screen_capture.lock().await;
                screen_capture.set_resolution(width, height).await?;
                Ok(None)
            }
            
            Message::Heartbeat { timestamp } => {
                // Echo heartbeat back
                Ok(Some(Message::Heartbeat { timestamp }))
            }
            
            _ => {
                tracing::warn!("Unhandled message: {:?}", message);
                Ok(None)
            }
        }
    }
    
    /// Get current screen frame
    pub async fn get_screen_frame(&self) -> Result<Vec<u8>, HostError> {
        let mut screen_capture = self.screen_capture.lock().await;
        screen_capture.capture_frame().await
    }
    
    /// Get partner ID
    pub fn partner_id(&self) -> &str {
        &self.partner_id
    }
    
    /// Check if host is running
    pub async fn is_running(&self) -> bool {
        *self.is_running.lock().await
    }
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("Host is already running")]
    AlreadyRunning,
    
    #[error("Host is not running")]
    NotRunning,
    
    #[error("Screen capture error: {0}")]
    ScreenCaptureError(String),
    
    #[error("Input simulation error: {0}")]
    InputError(String),
    
    #[error("File system error: {0}")]
    FileSystemError(String),
    
    #[error("Network error: {0}")]
    NetworkError(String),
}
