use std::sync::Arc;
use tokio::sync::Mutex;
use crate::protocol::{Message, NetworkManager, ConnectionState};

pub mod remote_session;
pub mod connection_manager;

pub use remote_session::RemoteSession;
pub use connection_manager::ConnectionManager;

/// Main client for connecting to remote computers
pub struct FreeViewerClient {
    connection_manager: Arc<Mutex<ConnectionManager>>,
    current_session: Arc<Mutex<Option<RemoteSession>>>,
}

impl FreeViewerClient {
    pub fn new() -> Self {
        Self {
            connection_manager: Arc::new(Mutex::new(ConnectionManager::new())),
            current_session: Arc::new(Mutex::new(None)),
        }
    }
    
    /// Connect to a remote computer
    pub async fn connect(&self, partner_id: String, password: String) -> Result<(), ClientError> {
        let mut connection_manager = self.connection_manager.lock().await;
        
        // Start connection process
        connection_manager.connect(partner_id.clone(), password).await?;
        
        // Create remote session
        let session = RemoteSession::new(partner_id);
        *self.current_session.lock().await = Some(session);
        
        Ok(())
    }
    
    /// Disconnect from current session
    pub async fn disconnect(&self) -> Result<(), ClientError> {
        let mut connection_manager = self.connection_manager.lock().await;
        connection_manager.disconnect().await?;
        
        *self.current_session.lock().await = None;
        
        Ok(())
    }
    
    /// Get current connection state
    pub async fn connection_state(&self) -> ConnectionState {
        let connection_manager = self.connection_manager.lock().await;
        connection_manager.state()
    }
    
    /// Send mouse input to remote computer
    pub async fn send_mouse_input(&self, x: f32, y: f32, button: Option<crate::protocol::MouseButton>, pressed: bool) -> Result<(), ClientError> {
        let connection_manager = self.connection_manager.lock().await;
        
        if button.is_some() {
            let message = Message::MouseClick { 
                x, y, 
                button: button.unwrap(), 
                pressed 
            };
            connection_manager.send_message(message).await?;
        } else {
            let message = Message::MouseMove { x, y };
            connection_manager.send_message(message).await?;
        }
        
        Ok(())
    }
    
    /// Send keyboard input to remote computer
    pub async fn send_keyboard_input(&self, key: String, pressed: bool, modifiers: crate::protocol::KeyModifiers) -> Result<(), ClientError> {
        let connection_manager = self.connection_manager.lock().await;
        
        let message = Message::KeyPress { key, pressed, modifiers };
        connection_manager.send_message(message).await?;
        
        Ok(())
    }
    
    /// Request file list from remote computer
    pub async fn request_file_list(&self, path: String) -> Result<Vec<crate::protocol::FileInfo>, ClientError> {
        let connection_manager = self.connection_manager.lock().await;
        
        let message = Message::FileListRequest { path };
        connection_manager.send_message(message).await?;
        
        // TODO: Wait for response and return file list
        // For now, return empty list
        Ok(Vec::new())
    }
    
    /// Start file transfer
    pub async fn transfer_file(&self, local_path: String, remote_path: String) -> Result<(), ClientError> {
        // TODO: Implement file transfer
        tracing::info!("Starting file transfer: {} -> {}", local_path, remote_path);
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),
    
    #[error("Authentication failed")]
    AuthenticationFailed,
    
    #[error("Not connected")]
    NotConnected,
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Protocol error: {0}")]
    ProtocolError(String),
    
    #[error("File transfer error: {0}")]
    FileTransferError(String),
}

impl From<connection_manager::ConnectionError> for ClientError {
    fn from(err: connection_manager::ConnectionError) -> Self {
        match err {
            connection_manager::ConnectionError::NetworkError(msg) => ClientError::NetworkError(msg),
            connection_manager::ConnectionError::NotConnected => ClientError::NotConnected,
            connection_manager::ConnectionError::AuthenticationFailed => ClientError::AuthenticationFailed,
            connection_manager::ConnectionError::Timeout => ClientError::NetworkError("Connection timeout".to_string()),
        }
    }
}
