use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes256Gcm, Key, Nonce,
};
use rand::RngCore;

pub mod authentication;
pub mod certificates;

pub use authentication::AuthManager;
pub use certificates::CertificateManager;

/// Security manager for encryption and authentication
pub struct SecurityManager {
    cipher: Option<Aes256Gcm>,
    auth_manager: AuthManager,
    cert_manager: CertificateManager,
}

impl SecurityManager {
    pub fn new() -> Self {
        Self {
            cipher: None,
            auth_manager: AuthManager::new(),
            cert_manager: CertificateManager::new(),
        }
    }
    
    /// Initialize encryption with a shared key
    pub fn init_encryption(&mut self, key: &[u8; 32]) -> Result<(), SecurityError> {
        let key = Key::<Aes256Gcm>::from_slice(key);
        self.cipher = Some(Aes256Gcm::new(key));
        Ok(())
    }
    
    /// Generate a new encryption key
    pub fn generate_key(&self) -> [u8; 32] {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        key
    }
    
    /// Encrypt data
    pub fn encrypt(&self, data: &[u8]) -> Result<Vec<u8>, SecurityError> {
        let cipher = self.cipher.as_ref()
            .ok_or(SecurityError::NotInitialized)?;
            
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher.encrypt(&nonce, data)
            .map_err(|e| SecurityError::EncryptionFailed(e.to_string()))?;
            
        // Prepend nonce to ciphertext
        let mut result = nonce.to_vec();
        result.extend_from_slice(&ciphertext);
        
        Ok(result)
    }
    
    /// Decrypt data
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>, SecurityError> {
        let cipher = self.cipher.as_ref()
            .ok_or(SecurityError::NotInitialized)?;
            
        if data.len() < 12 {
            return Err(SecurityError::InvalidData);
        }
        
        // Extract nonce and ciphertext
        let (nonce_bytes, ciphertext) = data.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);
        
        let plaintext = cipher.decrypt(nonce, ciphertext)
            .map_err(|e| SecurityError::DecryptionFailed(e.to_string()))?;
            
        Ok(plaintext)
    }
    
    /// Generate a session token
    pub fn generate_session_token(&self) -> String {
        let mut token = [0u8; 32];
        OsRng.fill_bytes(&mut token);
        hex::encode(token)
    }
    
    /// Validate session token
    pub fn validate_session_token(&self, token: &str) -> bool {
        // TODO: Implement proper token validation
        // For now, just check if it's a valid hex string of correct length
        token.len() == 64 && hex::decode(token).is_ok()
    }
    
    /// Hash password
    pub fn hash_password(&self, password: &str, salt: &[u8]) -> Vec<u8> {
        use sha2::{Sha256, Digest};
        let mut hasher = Sha256::new();
        hasher.update(password.as_bytes());
        hasher.update(salt);
        hasher.finalize().to_vec()
    }
    
    /// Generate salt
    pub fn generate_salt(&self) -> [u8; 32] {
        let mut salt = [0u8; 32];
        OsRng.fill_bytes(&mut salt);
        salt
    }
    
    /// Verify password
    pub fn verify_password(&self, password: &str, hash: &[u8], salt: &[u8]) -> bool {
        let computed_hash = self.hash_password(password, salt);
        computed_hash == hash
    }
}

impl Default for SecurityManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    #[error("Security manager not initialized")]
    NotInitialized,
    
    #[error("Encryption failed: {0}")]
    EncryptionFailed(String),
    
    #[error("Decryption failed: {0}")]
    DecryptionFailed(String),
    
    #[error("Invalid data format")]
    InvalidData,
    
    #[error("Authentication failed")]
    AuthenticationFailed,
    
    #[error("Certificate error: {0}")]
    CertificateError(String),
    
    #[error("Key generation failed")]
    KeyGenerationFailed,
}
