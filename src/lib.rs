// FreeViewer library - core modules for both GUI and daemon

pub mod capture;
pub mod client;
pub mod host;
pub mod protocol;
pub mod security;

// Re-export commonly used types
pub use client::FreeViewerClient;
pub use host::FreeViewerHost;
pub use protocol::Message;
pub use security::SecurityManager;
