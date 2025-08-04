use crate::security::SecurityManager;

/// Protocol-level encryption wrapper
pub struct ProtocolEncryption {
    security_manager: SecurityManager,
    is_enabled: bool,
}

impl ProtocolEncryption {
    pub fn new() -> Self {
        Self {
            security_manager: SecurityManager::new(),
            is_enabled: true,
        }
    }
    
    pub fn enable(&mut self, enable: bool) {
        self.is_enabled = enable;
    }
    
    pub fn is_enabled(&self) -> bool {
        self.is_enabled
    }
    
    pub fn init_encryption(&mut self, key: &[u8; 32]) -> Result<(), EncryptionError> {
        self.security_manager.init_encryption(key)
            .map_err(|e| EncryptionError::InitFailed(e.to_string()))
    }
    
    pub fn encrypt_data(&self, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if !self.is_enabled {
            return Ok(data.to_vec());
        }
        
        self.security_manager.encrypt(data)
            .map_err(|e| EncryptionError::EncryptFailed(e.to_string()))
    }
    
    pub fn decrypt_data(&self, data: &[u8]) -> Result<Vec<u8>, EncryptionError> {
        if !self.is_enabled {
            return Ok(data.to_vec());
        }
        
        self.security_manager.decrypt(data)
            .map_err(|e| EncryptionError::DecryptFailed(e.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EncryptionError {
    #[error("Encryption initialization failed: {0}")]
    InitFailed(String),
    
    #[error("Encryption failed: {0}")]
    EncryptFailed(String),
    
    #[error("Decryption failed: {0}")]
    DecryptFailed(String),
}
