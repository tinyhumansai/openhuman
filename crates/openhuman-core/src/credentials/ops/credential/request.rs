use serde::Deserialize;
use serde_json::Value;

use crate::credentials::jwt::decode_jwt_exp;
use crate::credentials::session_state::{
    local_session_user_id, user_id_from_jwt_claims, CredentialKind,
};

/// `auth.set_credential` parameters.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetCredentialRequest {
    /// The secret: a session JWT, an API key, or the local offline token.
    pub token: String,
    /// `"session"`, `"api-key"` or `"local"`. Defaults to classifying `token`
    /// by shape; an API key must be named explicitly.
    #[serde(default)]
    pub kind: Option<String>,
    /// The backend user id. Optional when `user` carries one or the JWT has a
    /// subject claim; required otherwise for a session.
    #[serde(default, alias = "user_id")]
    pub user_id: Option<String>,
    /// The user payload (the host's `/auth/me` answer, or the local user).
    #[serde(default)]
    pub user: Option<Value>,
    /// Normalized backend used by the host to validate this session.
    #[serde(default)]
    pub issuing_backend: Option<String>,
}

/// Reject session credentials shortly before their real expiry.
const EXPIRY_SKEW_SECS: i64 = 30;

pub(super) fn sanitize_user(user: Option<Value>) -> Option<Value> {
    match user {
        Some(Value::Object(map)) if map.is_empty() => None,
        Some(Value::Null) => None,
        other => other,
    }
}

pub(super) fn user_id_from_payload(user: Option<&Value>) -> Option<String> {
    user.and_then(crate::credentials::jwt::user_id_from_profile_payload)
}

pub(super) fn normalize_local_user(user: Value, local_user_id: &str) -> Value {
    let mut map = match user {
        Value::Object(map) => map,
        other => return other,
    };
    map.insert("id".to_string(), Value::String(local_user_id.to_string()));
    map.insert("_id".to_string(), Value::String(local_user_id.to_string()));
    Value::Object(map)
}

/// What `set_credential` resolved before touching any state.
pub(crate) struct Resolved {
    pub(crate) kind: CredentialKind,
    pub(crate) token: String,
    pub(crate) user_id: Option<String>,
    pub(crate) user: Option<Value>,
    pub(crate) expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub(crate) fn resolve(request: SetCredentialRequest) -> Result<Resolved, String> {
    let token = request.token.trim().to_string();
    if token.is_empty() {
        return Err("token is required".to_string());
    }
    let kind = match request
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        Some(raw) => CredentialKind::parse(raw).ok_or_else(|| {
            format!("unknown credential kind {raw:?}; expected session, api-key or local")
        })?,
        None => CredentialKind::classify(&token),
    };
    let explicit_user_id = request
        .user_id
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let user = sanitize_user(request.user);

    match kind {
        CredentialKind::ApiKey => Ok(Resolved {
            kind,
            token,
            user_id: None,
            user: None,
            expires_at: None,
        }),
        CredentialKind::Local => {
            let local_id = local_session_user_id();
            let user = user.ok_or_else(|| "local session requires a user payload".to_string())?;
            let user = normalize_local_user(user, &local_id);
            Ok(Resolved {
                kind,
                token,
                user_id: Some(local_id),
                user: Some(user),
                expires_at: None,
            })
        }
        CredentialKind::Session => {
            let expires_at = decode_jwt_exp(&token);
            if let Some(exp) = expires_at {
                if chrono::Utc::now() + chrono::Duration::seconds(EXPIRY_SKEW_SECS) >= exp {
                    return Err(format!(
                        "CREDENTIAL_EXPIRED: session token expired at {}; obtain a fresh one",
                        exp.to_rfc3339()
                    ));
                }
            }
            let user_id = explicit_user_id
                .or_else(|| user_id_from_payload(user.as_ref()))
                .or_else(|| user_id_from_jwt_claims(&token))
                .ok_or_else(|| {
                    "userId required: the token carries no subject claim and no user payload named one"
                        .to_string()
                })?;
            Ok(Resolved {
                kind,
                token,
                user_id: Some(user_id),
                user,
                expires_at,
            })
        }
    }
}
