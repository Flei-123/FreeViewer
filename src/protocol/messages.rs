use serde::{Deserialize, Serialize};
use crate::protocol::{Message, MouseButton, KeyModifiers, FileInfo};

/// Message utilities and helpers
pub struct MessageHandler;

impl MessageHandler {
    pub fn serialize_message(message: &Message) -> Result<Vec<u8>, MessageError> {
        bincode::serialize(message)
            .map_err(|e| MessageError::SerializationFailed(e.to_string()))
    }
    
    pub fn deserialize_message(data: &[u8]) -> Result<Message, MessageError> {
        bincode::deserialize(data)
            .map_err(|e| MessageError::DeserializationFailed(e.to_string()))
    }
    
    pub fn create_auth_request(id: String, password: String) -> Message {
        Message::AuthRequest { id, password }
    }
    
    pub fn create_auth_response(success: bool, session_token: Option<String>) -> Message {
        Message::AuthResponse { success, session_token }
    }
    
    pub fn create_screen_frame(data: Vec<u8>, width: u32, height: u32) -> Message {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
            
        Message::ScreenFrame { data, width, height, timestamp }
    }
    
    pub fn create_mouse_move(x: f32, y: f32) -> Message {
        Message::MouseMove { x, y }
    }
    
    pub fn create_mouse_click(x: f32, y: f32, button: MouseButton, pressed: bool) -> Message {
        Message::MouseClick { x, y, button, pressed }
    }
    
    pub fn create_key_press(key: String, pressed: bool, modifiers: KeyModifiers) -> Message {
        Message::KeyPress { key, pressed, modifiers }
    }
    
    pub fn create_heartbeat() -> Message {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
            
        Message::Heartbeat { timestamp }
    }
    
    pub fn create_file_list_request(path: String) -> Message {
        Message::FileListRequest { path }
    }
    
    pub fn create_file_list_response(files: Vec<FileInfo>) -> Message {
        Message::FileListResponse { files }
    }
    
    pub fn create_disconnect(reason: String) -> Message {
        Message::Disconnect { reason }
    }
    
    pub fn create_error(message: String) -> Message {
        Message::Error { message }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MessageError {
    #[error("Serialization failed: {0}")]
    SerializationFailed(String),
    
    #[error("Deserialization failed: {0}")]
    DeserializationFailed(String),
    
    #[error("Invalid message format")]
    InvalidFormat,
    
    #[error("Unsupported message type")]
    UnsupportedType,
}
