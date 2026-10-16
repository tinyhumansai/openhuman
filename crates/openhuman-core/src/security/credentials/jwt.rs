//! Session JWT load and `Authorization` helpers for the TinyHumans API.
//!
//! The backend issues a bare JWT as the session token. These helpers *read*
//! it — they never verify it. The backend stays the authority on validity: a
//! token revoked before its `exp` still returns 401, and callers must handle
//! that. Reading `exp` locally is only an optimisation that avoids sending a
//! request with a token already known to be dead.
//!
//! Parsing is pure and has no backend dependency, which is why it lives in the
//! core rather than behind a host crate: `security::credentials`
//! needs it on a core that has no TinyHumans connection at all.
//!
//! What stays OpenHuman-specific is *where the token lives*: the credentials
//! store, keyring, and auth-profile names below.

use base64::Engine;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Format a token as an `Authorization: Bearer …` header value.
///
/// Surrounding whitespace is trimmed — tokens pasted by hand or read from a
/// file routinely carry a trailing newline, and the backend rejects the header
/// if it survives. Interior whitespace is left alone: it cannot appear in a
/// well-formed JWT, so trimming it would mask a malformed token rather than
/// fix one.
pub fn bearer_authorization_value(token: &str) -> String {
    format!("Bearer {}", token.trim())
}

/// Decode a JWT's payload without verifying the signature.
///
/// Returns `None` for anything that is not a JWT with a base64url payload
/// holding JSON — including the non-JWT sentinels hosts store for offline or
/// local sessions, which must not panic here.
pub fn decode_jwt_payload(token: &str) -> Option<Value> {
    // JWT = header.payload.signature (base64url, no padding). Only the payload
    // segment is needed. Padded input is accepted as a fallback because not
    // every issuer omits padding.
    let payload_b64 = token.trim().split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(payload_b64))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Read a JWT's `exp` claim as a Unix timestamp in seconds. `exp` is a
/// NumericDate, so both integer and float encodings are accepted.
pub fn decode_jwt_exp_unix(token: &str) -> Option<i64> {
    decode_jwt_payload(token)?
        .get("exp")
        .and_then(|value| value.as_i64().or_else(|| value.as_f64().map(|f| f as i64)))
}

pub use crate::security::credentials::session_support::get_session_token;
pub use crate::security::credentials::{APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};

/// Best-effort decode of a JWT's `exp` (expiry) claim into a UTC timestamp.
///
/// The backend app-session token is a JWT but is stored bare — the client
/// historically recorded `expires_at: None` and so blindly sent requests with a
/// token it could have known was dead, generating doomed 401s (Sentry
/// TAURI-RUST-8WY (team usage), 8WZ (current plan); #3297).
/// Decoding `exp` at store time lets `require_live_session_token` reject an
/// expired token locally instead of round-tripping to a guaranteed 401.
///
/// This does NOT verify the signature — the client only needs to *read* `exp`;
/// the backend stays the authority on validity (a token revoked before its `exp`
/// still 401s, caught by the `flatten_authed_error` net). Returns `None` for any
/// non-JWT / malformed / `exp`-less token, in which case expiry tracking
/// degrades to the previous behaviour (no local precheck).
///
/// [`decode_jwt_exp_unix`] wrapped in the `chrono` type the credentials store
/// already uses.
pub fn decode_jwt_exp(token: &str) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(decode_jwt_exp_unix(token)?, 0)
}

fn user_id_from_object(obj: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["id", "_id", "userId"] {
        if let Some(s) = obj.get(key).and_then(|x| x.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

/// Best-effort extraction of a user ID from an authenticated profile payload.
///
/// This function handles various envelope formats, including raw user objects
/// or those nested under `data` or `user` keys.
pub fn user_id_from_profile_payload(payload: &Value) -> Option<String> {
    let obj = payload.as_object()?;
    if let Some(data) = obj.get("data").and_then(|v| v.as_object()) {
        return user_id_from_object(data).or_else(|| {
            data.get("user")
                .and_then(|u| u.as_object())
                .and_then(user_id_from_object)
        });
    }

    user_id_from_object(obj).or_else(|| {
        obj.get("user")
            .and_then(|u| u.as_object())
            .and_then(user_id_from_object)
    })
}

#[cfg(test)]
#[path = "jwt_tests.rs"]
mod tests;
