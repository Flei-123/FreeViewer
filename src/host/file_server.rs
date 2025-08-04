use crate::protocol::FileInfo;
use std::path::Path;

/// Handles file system operations for remote file access
pub struct FileServer {
    is_running: bool,
}

impl FileServer {
    pub fn new() -> Self {
        Self {
            is_running: false,
        }
    }
    
    pub async fn start(&mut self) -> Result<(), super::HostError> {
        self.is_running = true;
        tracing::info!("File server started");
        Ok(())
    }
    
    pub async fn stop(&mut self) -> Result<(), super::HostError> {
        self.is_running = false;
        tracing::info!("File server stopped");
        Ok(())
    }
    
    pub async fn list_files(&mut self, path: String) -> Result<Vec<FileInfo>, super::HostError> {
        if !self.is_running {
            return Err(super::HostError::FileSystemError("File server not running".to_string()));
        }
        
        let path = Path::new(&path);
        let mut files = Vec::new();
        
        match std::fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries.filter_map(|e| e.ok()) {
                    let entry_path = entry.path();
                    let name = entry_path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("?")
                        .to_string();
                    
                    let metadata = entry.metadata().ok();
                    let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                    let is_directory = entry_path.is_dir();
                    let modified = metadata
                        .and_then(|m| m.modified().ok())
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0);
                    
                    files.push(FileInfo {
                        name,
                        path: entry_path.to_string_lossy().to_string(),
                        size,
                        is_directory,
                        modified,
                    });
                }
            }
            Err(e) => {
                return Err(super::HostError::FileSystemError(format!("Failed to read directory: {}", e)));
            }
        }
        
        // Sort: directories first, then by name
        files.sort_by(|a, b| {
            match (a.is_directory, b.is_directory) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.name.cmp(&b.name),
            }
        });
        
        tracing::debug!("Listed {} files in {}", files.len(), path.display());
        Ok(files)
    }
    
    pub fn is_running(&self) -> bool {
        self.is_running
    }
}
