use std::collections::HashMap;
use tokio::sync::RwLock;
use uuid::Uuid;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::password_hash::{rand_core::OsRng, SaltString};

/// Authentication manager for handling user login and sessions
pub struct AuthManager {
    sessions: RwLock<HashMap<String, Session>>,
    users: RwLock<HashMap<String, User>>,
    argon2: Argon2<'static>,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub token: String,
    pub user_id: String,
    pub created_at: std::time::SystemTime,
    pub expires_at: std::time::SystemTime,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: String,
    pub password_hash: String,
    pub created_at: std::time::SystemTime,
    pub last_login: Option<std::time::SystemTime>,
    pub is_active: bool,
}

impl AuthManager {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
            users: RwLock::new(HashMap::new()),
            argon2: Argon2::default(),
        }
    }
    
    /// Create a new user with hashed password
    pub async fn create_user(&self, id: String, password: String) -> Result<(), AuthError> {
        let salt = SaltString::generate(&mut OsRng);
        let password_hash = self.argon2
            .hash_password(password.as_bytes(), &salt)
            .map_err(|e| AuthError::HashingFailed(e.to_string()))?
            .to_string();
        
        let user = User {
            id: id.clone(),
            password_hash,
            created_at: std::time::SystemTime::now(),
            last_login: None,
            is_active: true,
        };
        
        let mut users = self.users.write().await;
        if users.contains_key(&id) {
            return Err(AuthError::UserExists);
        }
        
        users.insert(id, user);
        Ok(())
    }
    
    /// Authenticate user and create session
    pub async fn authenticate(&self, id: String, password: String) -> Result<String, AuthError> {
        let mut users = self.users.write().await;
        let user = users.get_mut(&id).ok_or(AuthError::InvalidCredentials)?;
        
        if !user.is_active {
            return Err(AuthError::UserInactive);
        }
        
        // Verify password
        let parsed_hash = PasswordHash::new(&user.password_hash)
            .map_err(|e| AuthError::HashingFailed(e.to_string()))?;
        
        self.argon2
            .verify_password(password.as_bytes(), &parsed_hash)
            .map_err(|_| AuthError::InvalidCredentials)?;
        
        // Update last login
        user.last_login = Some(std::time::SystemTime::now());
        
        // Create session
        let session_token = Uuid::new_v4().to_string();
        let session = Session {
            token: session_token.clone(),
            user_id: id,
            created_at: std::time::SystemTime::now(),
            expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(3600), // 1 hour
            is_active: true,
        };
        
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_token.clone(), session);
        
        Ok(session_token)
    }
    
    /// Validate session token
    pub async fn validate_session(&self, token: &str) -> Result<Session, AuthError> {
        let sessions = self.sessions.read().await;
        let session = sessions.get(token).ok_or(AuthError::InvalidSession)?;
        
        if !session.is_active {
            return Err(AuthError::SessionInactive);
        }
        
        if session.expires_at < std::time::SystemTime::now() {
            return Err(AuthError::SessionExpired);
        }
        
        Ok(session.clone())
    }
    
    /// Revoke session
    pub async fn revoke_session(&self, token: &str) -> Result<(), AuthError> {
        let mut sessions = self.sessions.write().await;
        if let Some(session) = sessions.get_mut(token) {
            session.is_active = false;
        }
        Ok(())
    }
    
    /// Clean up expired sessions
    pub async fn cleanup_expired_sessions(&self) {
        let mut sessions = self.sessions.write().await;
        let now = std::time::SystemTime::now();
        
        sessions.retain(|_, session| {
            session.is_active && session.expires_at > now
        });
    }
    
    /// Generate temporary access code for quick connections
    pub async fn generate_access_code(&self, user_id: String) -> Result<String, AuthError> {
        let code = format!("{:06}", rand::random::<u32>() % 1_000_000);
        
        // Store code with short expiration (5 minutes)
        let session = Session {
            token: code.clone(),
            user_id,
            created_at: std::time::SystemTime::now(),
            expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(300),
            is_active: true,
        };
        
        let mut sessions = self.sessions.write().await;
        sessions.insert(code.clone(), session);
        
        Ok(code)
    }
}

impl Default for AuthManager {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    
    #[error("User already exists")]
    UserExists,
    
    #[error("User is inactive")]
    UserInactive,
    
    #[error("Invalid session")]
    InvalidSession,
    
    #[error("Session is inactive")]
    SessionInactive,
    
    #[error("Session has expired")]
    SessionExpired,
    
    #[error("Password hashing failed: {0}")]
    HashingFailed(String),
    
    #[error("Database error: {0}")]
    DatabaseError(String),
}
