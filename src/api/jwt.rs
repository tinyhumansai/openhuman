//! Session JWT load and `Authorization` helpers for the TinyHumans API.

pub use crate::openhuman::credentials::session_support::get_session_token;
pub use crate::openhuman::credentials::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

/// Value for `Authorization: Bearer …` (matches backend expectations).
pub fn bearer_authorization_value(token: &str) -> String {
    format!("Bearer {}", token.trim())
}
