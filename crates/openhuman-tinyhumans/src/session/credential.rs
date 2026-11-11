//! The credential a host hands to the core, and the pure JWT / payload helpers
//! the session flow needs before it can hand one over.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Which kind of backend credential a token is. The `as_str` values are the
/// wire vocabulary of the core's `auth.set_credential` / `auth.get_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CredentialKind {
    /// A TinyHumans session JWT — `Authorization: Bearer` on every backend route.
    Session,
    /// A TinyHumans API key — bearer on managed inference, `x-api-key` on REST.
    ApiKey,
    /// The offline local session: a fake JWT whose signature segment is
    /// literally `local`. Never sent to a backend.
    Local,
}

impl CredentialKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::ApiKey => "api-key",
            Self::Local => "local",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "session" => Some(Self::Session),
            "api-key" => Some(Self::ApiKey),
            "local" => Some(Self::Local),
            _ => None,
        }
    }
}

/// A backend credential together with what is locally knowable about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub kind: CredentialKind,
    pub secret: String,
    /// The JWT `exp` claim, when the token carries one. API keys and local
    /// sessions never do.
    pub expires_at: Option<DateTime<Utc>>,
}

impl Credential {
    pub fn session(token: impl Into<String>) -> Self {
        let secret = token.into().trim().to_string();
        let expires_at = decode_jwt_exp(&secret);
        Self {
            kind: CredentialKind::Session,
            secret,
            expires_at,
        }
    }

    pub fn api_key(key: impl Into<String>) -> Self {
        Self {
            kind: CredentialKind::ApiKey,
            secret: key.into().trim().to_string(),
            expires_at: None,
        }
    }

    pub fn local(token: impl Into<String>) -> Self {
        Self {
            kind: CredentialKind::Local,
            secret: token.into().trim().to_string(),
            expires_at: None,
        }
    }

    /// Classify a bare token the way the core always has: a three-segment
    /// token whose third segment is literally `local` is the offline session,
    /// anything else is a session JWT. API keys are never classified from
    /// shape — a host that holds one knows it does.
    pub fn classify(token: &str) -> Self {
        if is_local_session_token(token) {
            Self::local(token)
        } else {
            Self::session(token)
        }
    }

    pub fn is_local(&self) -> bool {
        self.kind == CredentialKind::Local
    }
}

/// Whether `token` is the offline local session shape.
pub fn is_local_session_token(token: &str) -> bool {
    let trimmed = token.trim();
    let mut parts = trimmed.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(_), Some(_), Some("local"), None)
    )
}

/// Best-effort decode of a JWT's `exp` claim. Does not verify the signature —
/// the backend stays the authority on validity; this only lets a host refuse
/// to store a token it can already see is dead.
pub fn decode_jwt_exp(token: &str) -> Option<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(tinyhumans_sdk::jwt::decode_jwt_exp_unix(token)?, 0)
}

/// `Some(exp)` when the token's `exp` is still in the future at `now`;
/// `None` for an expired token **or** a token with no decodable `exp`.
pub fn jwt_is_live(token: &str, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let exp = decode_jwt_exp(token)?;
    (exp > now).then_some(exp)
}

/// The subject of a JWT, read from its payload claims without verification.
/// Checked in order: `sub`, `userId`, `user_id`, `_id`, `id`.
pub fn user_id_from_jwt_claims(token: &str) -> Option<String> {
    let claims = tinyhumans_sdk::jwt::decode_jwt_payload(token)?;
    let obj = claims.as_object()?;
    ["sub", "userId", "user_id", "_id", "id"]
        .iter()
        .find_map(|key| obj.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn user_id_from_object(obj: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["id", "_id", "userId"] {
        if let Some(value) = obj.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// The user id inside a `/auth/me`-shaped payload: top-level `id`/`_id`/
/// `userId`, or the same under `data` or `user`. Same rules as the core's
/// `openhuman_embed::__host::credentials::jwt::user_id_from_profile_payload`.
pub fn user_id_from_profile_payload(payload: &Value) -> Option<String> {
    let obj = payload.as_object()?;
    if let Some(data) = obj.get("data").and_then(Value::as_object) {
        return user_id_from_object(data).or_else(|| {
            data.get("user")
                .and_then(Value::as_object)
                .and_then(user_id_from_object)
        });
    }
    user_id_from_object(obj).or_else(|| {
        obj.get("user")
            .and_then(Value::as_object)
            .and_then(user_id_from_object)
    })
}

#[cfg(test)]
#[path = "credential_tests.rs"]
mod tests;
