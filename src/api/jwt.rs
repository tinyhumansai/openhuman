//! Session JWT load and `Authorization` helpers for the TinyHumans API.
//!
//! Parsing and header formatting live in the vendored SDK
//! (`tinyhumans_sdk::jwt`) — they are properties of the backend's token, not of
//! this client, and every host needs them. Re-exported here so existing call
//! sites keep one import path.
//!
//! What stays OpenHuman-specific is *where the token lives*: the credentials
//! store, keyring, and auth-profile names below.

use chrono::{DateTime, Utc};

pub use tinyhumans_sdk::jwt::{bearer_authorization_value, decode_jwt_payload};

pub use crate::openhuman::security::credentials::session_support::get_session_token;
pub use crate::openhuman::security::credentials::{
    APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};

/// Best-effort decode of a JWT's `exp` (expiry) claim into a UTC timestamp.
///
/// The backend app-session token is a JWT but is stored bare — the client
/// historically recorded `expires_at: None` and so blindly sent requests with a
/// token it could have known was dead, generating doomed 401s (Sentry
/// TAURI-RUST-8WY `/teams/me/usage`, 8WZ `/payments/stripe/currentPlan`; #3297).
/// Decoding `exp` at store time lets `require_live_session_token` reject an
/// expired token locally instead of round-tripping to a guaranteed 401.
///
/// This does NOT verify the signature — the client only needs to *read* `exp`;
/// the backend stays the authority on validity (a token revoked before its `exp`
/// still 401s, caught by the `flatten_authed_error` net). Returns `None` for any
/// non-JWT / malformed / `exp`-less token, in which case expiry tracking
/// degrades to the previous behaviour (no local precheck).
///
/// The SDK returns Unix seconds so it needs no datetime dependency; this wraps
/// that in the `chrono` type the credentials store already uses.
pub fn decode_jwt_exp(token: &str) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(tinyhumans_sdk::jwt::decode_jwt_exp_unix(token)?, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn jwt_with_payload(payload_json: &str) -> String {
        use base64::Engine;
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
    }

    // The SDK owns the parsing rules and tests them directly. What matters here
    // is that the `chrono` conversion this crate depends on stays correct.
    #[test]
    fn decode_jwt_exp_reads_integer_exp() {
        let token = jwt_with_payload(r#"{"sub":"u1","exp":1700000000}"#);
        assert_eq!(
            decode_jwt_exp(&token),
            DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
        );
    }

    #[test]
    fn decode_jwt_exp_reads_float_exp() {
        let token = jwt_with_payload(r#"{"exp":1700000000.0}"#);
        assert_eq!(
            decode_jwt_exp(&token),
            DateTime::<Utc>::from_timestamp(1_700_000_000, 0)
        );
    }

    #[test]
    fn decode_jwt_exp_none_when_exp_absent() {
        let token = jwt_with_payload(r#"{"sub":"u1"}"#);
        assert_eq!(decode_jwt_exp(&token), None);
    }

    #[test]
    fn decode_jwt_exp_none_for_non_jwt_or_garbage() {
        assert_eq!(decode_jwt_exp("not-a-jwt"), None);
        assert_eq!(decode_jwt_exp(""), None);
        assert_eq!(decode_jwt_exp("a.b"), None);
        // Local offline session sentinel (not a JWT) must not panic.
        assert_eq!(decode_jwt_exp("local-session-xyz"), None);
    }
}
