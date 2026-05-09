//! Session JWT load and `Authorization` helpers for the TinyHumans API.

pub use crate::openhuman::credentials::session_support::get_session_token;
pub use crate::openhuman::credentials::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

/// Value for `Authorization: Bearer …` (matches backend expectations).
pub fn bearer_authorization_value(token: &str) -> String {
    format!("Bearer {}", token.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bearer_authorization_value() {
        // Standard token
        assert_eq!(bearer_authorization_value("my_token"), "Bearer my_token");

        // Token with leading/trailing spaces
        assert_eq!(
            bearer_authorization_value("  spaced_token  "),
            "Bearer spaced_token"
        );

        // Empty string
        assert_eq!(bearer_authorization_value(""), "Bearer ");

        // Whitespace only string
        assert_eq!(bearer_authorization_value("   "), "Bearer ");

        // Token with internal spaces (should not be trimmed)
        assert_eq!(
            bearer_authorization_value("token with spaces"),
            "Bearer token with spaces"
        );
    }
}
