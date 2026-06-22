//! Session JWT load and `Authorization` helpers for the TinyHumans API.

use base64::Engine;
use chrono::{DateTime, Utc};

pub use crate::openhuman::credentials::session_support::get_session_token;
pub use crate::openhuman::credentials::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

/// Value for `Authorization: Bearer …` (matches backend expectations).
pub fn bearer_authorization_value(token: &str) -> String {
    format!("Bearer {}", token.trim())
}

/// Best-effort decode of a JWT payload without verifying the signature.
pub fn decode_jwt_payload(token: &str) -> Option<serde_json::Value> {
    // JWT = header.payload.signature (base64url, no padding). Only the payload
    // segment is needed.
    let payload_b64 = token.trim().split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

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
pub fn decode_jwt_exp(token: &str) -> Option<DateTime<Utc>> {
    let claims = decode_jwt_payload(token)?;
    // `exp` is a NumericDate (seconds since epoch); accept int or float shapes.
    let exp = claims
        .get("exp")
        .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))?;
    DateTime::<Utc>::from_timestamp(exp, 0)
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

    fn jwt_with_payload(payload_json: &str) -> String {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(payload_json);
        // header + signature are irrelevant to `decode_jwt_exp` (no verification).
        format!("eyJhbGciOiJIUzI1NiJ9.{payload}.sig")
    }

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
        assert_eq!(decode_jwt_exp("a.b"), None); // payload "b" isn't valid base64 JSON
                                                 // local offline session sentinel (not a JWT) must not panic / must be None
        assert_eq!(decode_jwt_exp("local-session-xyz"), None);
    }
}
