use serde::{Deserialize, Serialize};

pub mod network;
pub mod encryption;
pub mod messages;

pub use network::NetworkManager;
pub use encryption::SecurityManager;
pub use messages::*;

/// The main protocol version
pub const PROTOCOL_VERSION: u32 = 1;

/// Message types for the FreeViewer protocol
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    // Authentication
    AuthRequest { id: String, password: String },
    AuthResponse { success: bool, session_token: Option<String> },
    
    // Screen sharing
    ScreenFrame { data: Vec<u8>, width: u32, height: u32, timestamp: u64 },
    ScreenResolution { width: u32, height: u32 },
    
    // Input events
    MouseMove { x: f32, y: f32 },
    MouseClick { x: f32, y: f32, button: MouseButton, pressed: bool },
    MouseWheel { delta_x: f32, delta_y: f32 },
    KeyPress { key: String, pressed: bool, modifiers: KeyModifiers },
    
    // File transfer
    FileListRequest { path: String },
    FileListResponse { files: Vec<FileInfo> },
    FileTransferStart { path: String, size: u64 },
    FileTransferChunk { id: u64, offset: u64, data: Vec<u8> },
    FileTransferComplete { id: u64 },
    FileTransferError { id: u64, error: String },
    
    // Clipboard
    ClipboardSync { content: String },
    
    // System
    Heartbeat { timestamp: u64 },
    Disconnect { reason: String },
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Other(u8),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyModifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub is_directory: bool,
    pub modified: u64, // Unix timestamp
}

/// Connection state
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Authenticating,
    Connected,
    Error(String),
}

/// Protocol configuration
#[derive(Debug, Clone)]
pub struct ProtocolConfig {
    pub use_encryption: bool,
    pub compression_level: u8, // 0-9
    pub heartbeat_interval: std::time::Duration,
    pub connection_timeout: std::time::Duration,
    pub max_frame_rate: u32,
    pub max_file_chunk_size: usize,
}

impl Default for ProtocolConfig {
    fn default() -> Self {
        Self {
            use_encryption: true,
            compression_level: 6,
            heartbeat_interval: std::time::Duration::from_secs(5),
            connection_timeout: std::time::Duration::from_secs(30),
            max_frame_rate: 30,
            max_file_chunk_size: 64 * 1024, // 64KB
        }
    }
}
