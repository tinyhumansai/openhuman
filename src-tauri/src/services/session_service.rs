use crate::models::auth::{Session, User};
use crate::utils::config::{APP_IDENTIFIER, KEYCHAIN_SERVICE};
use serde::{Deserialize, Serialize};
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

/// Session data stored in keychain
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredSession {
    token: String,
    user_id: String,
    user: Option<User>,
    created_at: u64,
    expires_at: Option<u64>,
}

/// Service for managing user sessions with secure storage
pub struct SessionService {
    /// In-memory cache of the current session
    cached_session: RwLock<Option<StoredSession>>,
}

impl SessionService {
    /// Create a new SessionService instance
    pub fn new() -> Self {
        let service = Self {
            cached_session: RwLock::new(None),
        };
        // Try to load existing session from keychain
        if let Ok(session) = service.load_from_keychain() {
            if let Ok(mut cache) = service.cached_session.write() {
                *cache = Some(session);
            }
        }
        service
    }

    /// Store a new session
    pub fn store_session(&self, token: &str, user: &User) -> Result<(), String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();

        let session = StoredSession {
            token: token.to_string(),
            user_id: user.id.clone(),
            user: Some(user.clone()),
            created_at: now,
            expires_at: None, // Can be set from token if needed
        };

        // Store in keychain
        self.save_to_keychain(&session)?;

        // Update cache
        if let Ok(mut cache) = self.cached_session.write() {
            *cache = Some(session);
        }

        Ok(())
    }

    /// Get the current session token
    pub fn get_token(&self) -> Option<String> {
        self.cached_session
            .read()
            .ok()
            .and_then(|cache| cache.as_ref().map(|s| s.token.clone()))
    }

    /// Get the current session
    #[allow(dead_code)]
    pub fn get_session(&self) -> Option<Session> {
        self.cached_session.read().ok().and_then(|cache| {
            cache.as_ref().map(|s| Session {
                token: s.token.clone(),
                user_id: s.user_id.clone(),
                created_at: s.created_at,
                expires_at: s.expires_at,
            })
        })
    }

    /// Get the current user
    pub fn get_user(&self) -> Option<User> {
        self.cached_session
            .read()
            .ok()
            .and_then(|cache| cache.as_ref().and_then(|s| s.user.clone()))
    }

    /// Check if there's an active session
    pub fn is_authenticated(&self) -> bool {
        self.cached_session
            .read()
            .ok()
            .map(|cache| cache.is_some())
            .unwrap_or(false)
    }

    /// Clear the current session (logout)
    pub fn clear_session(&self) -> Result<(), String> {
        // Clear from keychain
        self.delete_from_keychain()?;

        // Clear cache
        if let Ok(mut cache) = self.cached_session.write() {
            *cache = None;
        }

        Ok(())
    }

    /// Save session to OS keychain
    fn save_to_keychain(&self, session: &StoredSession) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, APP_IDENTIFIER)
            .map_err(|e| format!("Failed to create keyring entry: {}", e))?;

        let json = serde_json::to_string(session)
            .map_err(|e| format!("Failed to serialize session: {}", e))?;

        entry
            .set_password(&json)
            .map_err(|e| format!("Failed to store in keychain: {}", e))?;

        Ok(())
    }

    /// Load session from OS keychain
    fn load_from_keychain(&self) -> Result<StoredSession, String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, APP_IDENTIFIER)
            .map_err(|e| format!("Failed to create keyring entry: {}", e))?;

        let json = entry
            .get_password()
            .map_err(|e| format!("Failed to load from keychain: {}", e))?;

        let session: StoredSession = serde_json::from_str(&json)
            .map_err(|e| format!("Failed to deserialize session: {}", e))?;

        Ok(session)
    }

    /// Delete session from OS keychain
    fn delete_from_keychain(&self) -> Result<(), String> {
        let entry = keyring::Entry::new(KEYCHAIN_SERVICE, APP_IDENTIFIER)
            .map_err(|e| format!("Failed to create keyring entry: {}", e))?;

        // Ignore error if entry doesn't exist
        let _ = entry.delete_credential();

        Ok(())
    }
}

impl Default for SessionService {
    fn default() -> Self {
        Self::new()
    }
}
