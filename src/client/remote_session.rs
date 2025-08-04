use std::sync::Arc;
use tokio::sync::Mutex;
use crate::protocol::Message;

/// Represents an active remote session
pub struct RemoteSession {
    partner_id: String,
    is_active: bool,
    session_token: Option<String>,
}

impl RemoteSession {
    pub fn new(partner_id: String) -> Self {
        Self {
            partner_id,
            is_active: false,
            session_token: None,
        }
    }
    
    pub fn partner_id(&self) -> &str {
        &self.partner_id
    }
    
    pub fn is_active(&self) -> bool {
        self.is_active
    }
    
    pub async fn start(&mut self, session_token: String) -> Result<(), SessionError> {
        self.session_token = Some(session_token);
        self.is_active = true;
        tracing::info!("Remote session started with partner: {}", self.partner_id);
        Ok(())
    }
    
    pub async fn stop(&mut self) -> Result<(), SessionError> {
        self.is_active = false;
        self.session_token = None;
        tracing::info!("Remote session stopped");
        Ok(())
    }
    
    pub async fn send_message(&self, _message: Message) -> Result<(), SessionError> {
        if !self.is_active {
            return Err(SessionError::SessionNotActive);
        }
        
        // TODO: Send message through network layer
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("Session is not active")]
    SessionNotActive,
    
    #[error("Network error: {0}")]
    NetworkError(String),
    
    #[error("Authentication failed")]
    AuthError,
}
