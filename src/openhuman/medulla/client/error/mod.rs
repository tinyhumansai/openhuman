//! Error type for the Medulla client.

use serde_json::Value;

impl ClientError {
    /// The backend `errorCode`, when this is an [`ClientError::Api`] error.
    pub fn error_code(&self) -> Option<&str> {
        match self {
            ClientError::Api { error_code, .. } => error_code.as_deref(),
            _ => None,
        }
    }

    /// HTTP status code, when available.
    pub fn status(&self) -> Option<u16> {
        match self {
            ClientError::Api { status, .. } => *status,
            _ => None,
        }
    }

    /// Whether the backend reported an expired token (`TOKEN_EXPIRED`).
    pub fn is_token_expired(&self) -> bool {
        self.error_code() == Some("TOKEN_EXPIRED")
    }

    /// Whether this error should route a front end to the login screen rather
    /// than falling back silently.
    ///
    /// True for an expired token or an HTTP 401/403 (rejected credentials). Every
    /// front end should branch on this so authentication failures are handled
    /// identically.
    pub fn is_auth_error(&self) -> bool {
        self.is_token_expired() || matches!(self.status(), Some(401) | Some(403))
    }
}

mod types;
pub use types::ClientError;
pub use types::Result;
