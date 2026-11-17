//! How a session owner reaches *its* core. The Tauri shell talks JSON-RPC
//! over loopback HTTP; the TUI holds an in-process `CoreRuntime`. Both are
//! just "invoke a method with params", so that is the whole trait.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::session::credential::{Credential, CredentialKind};

const LOG_PREFIX: &str = "[session][link]";

pub const AUTH_SET_CREDENTIAL: &str = "openhuman.auth_set_credential";
pub const AUTH_CLEAR_CREDENTIAL: &str = "openhuman.auth_clear_credential";
pub const AUTH_GET_STATE: &str = "openhuman.auth_get_state";
pub const AUTH_GET_SESSION_TOKEN: &str = "openhuman.auth_get_session_token";
pub const CONFIG_RESOLVE_API_URL: &str = "openhuman.config_resolve_api_url";

/// An RPC channel into one core. `invoke` returns the JSON-RPC `result`
/// (possibly wrapped in the core's `{result, logs}` envelope) or the error
/// message.
#[async_trait]
pub trait CoreLink: Send + Sync + 'static {
    async fn invoke(&self, method: &str, params: Value) -> Result<Value, String>;
}

/// What the core reports about the credential it holds — the shape of
/// `auth.get_state`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoreAuthState {
    #[serde(default)]
    pub is_authenticated: bool,
    /// `"session"`, `"api-key"`, `"local"`, or absent when signed out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(default)]
    pub user: Option<Value>,
    #[serde(default)]
    pub profile_id: Option<String>,
    /// RFC3339 expiry recorded from the JWT `exp`, when the core knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Issuing backend for a bound session; absent for legacy sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub issuing_backend: Option<String>,
}

impl CoreAuthState {
    pub fn kind(&self) -> Option<CredentialKind> {
        self.credential.as_deref().and_then(CredentialKind::parse)
    }
}

/// Peel the core's `{result, logs}` / `{data}` envelopes off an RPC result —
/// the same walk as `openhuman_rpc::unwrap_rpc`, copied so this crate stays
/// free of core-side dependencies.
pub fn unwrap_envelope(mut value: &Value) -> &Value {
    loop {
        match value.get("result").or_else(|| value.get("data")) {
            Some(next) => value = next,
            None => return value,
        }
    }
}

/// `auth.set_credential`: hand `credential` (and what is known about its
/// user) to the core, which persists it and runs its own credential-set side
/// effects. Returns the core's resulting auth state.
pub async fn push_credential<L: CoreLink + ?Sized>(
    link: &L,
    credential: &Credential,
    user_id: Option<&str>,
    user: Option<&Value>,
) -> Result<CoreAuthState, String> {
    push_bound_credential(link, credential, user_id, user, None).await
}

/// Hand over a credential with its issuing backend. Existing unbound callers
/// remain compatible, and the core retains a prior association on refresh.
pub async fn push_bound_credential<L: CoreLink + ?Sized>(
    link: &L,
    credential: &Credential,
    user_id: Option<&str>,
    user: Option<&Value>,
    issuing_backend: Option<&str>,
) -> Result<CoreAuthState, String> {
    let mut params = json!({
        "token": credential.secret,
        "kind": credential.kind.as_str(),
    });
    if let Some(backend) = issuing_backend {
        params["issuingBackend"] = Value::String(backend.to_string());
    }
    if let Some(user_id) = user_id.map(str::trim).filter(|s| !s.is_empty()) {
        params["userId"] = Value::String(user_id.to_string());
    }
    if let Some(user) = user {
        params["user"] = user.clone();
    }
    log::debug!(
        "{LOG_PREFIX} {AUTH_SET_CREDENTIAL} kind={} has_user_id={} has_user={}",
        credential.kind.as_str(),
        user_id.is_some(),
        user.is_some()
    );
    let value = link.invoke(AUTH_SET_CREDENTIAL, params).await?;
    serde_json::from_value(unwrap_envelope(&value).clone())
        .map_err(|e| format!("{AUTH_SET_CREDENTIAL} returned an unexpected shape: {e}"))
}

/// `auth.clear_credential`: drop the credential of `kind`, or every
/// credential when `None`.
pub async fn clear_credential<L: CoreLink + ?Sized>(
    link: &L,
    kind: Option<CredentialKind>,
) -> Result<(), String> {
    let params = match kind {
        Some(kind) => json!({ "kind": kind.as_str() }),
        None => json!({}),
    };
    log::debug!(
        "{LOG_PREFIX} {AUTH_CLEAR_CREDENTIAL} kind={}",
        kind.map(CredentialKind::as_str).unwrap_or("all")
    );
    link.invoke(AUTH_CLEAR_CREDENTIAL, params).await.map(|_| ())
}

/// `auth.get_state`.
pub async fn core_auth_state<L: CoreLink + ?Sized>(link: &L) -> Result<CoreAuthState, String> {
    let value = link.invoke(AUTH_GET_STATE, json!({})).await?;
    serde_json::from_value(unwrap_envelope(&value).clone())
        .map_err(|e| format!("{AUTH_GET_STATE} returned an unexpected shape: {e}"))
}

/// `auth.get_session_token`: the stored session secret, if any.
pub async fn core_session_token<L: CoreLink + ?Sized>(link: &L) -> Result<Option<String>, String> {
    let value = link.invoke(AUTH_GET_SESSION_TOKEN, json!({})).await?;
    Ok(unwrap_envelope(&value)
        .get("token")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string))
}

/// `config.resolve_api_url`: the backend base URL the core is configured for,
/// so the session owner and the core agree on which backend they talk to.
pub async fn resolve_backend_url<L: CoreLink + ?Sized>(link: &L) -> Result<String, String> {
    let value = link.invoke(CONFIG_RESOLVE_API_URL, json!({})).await?;
    let value = unwrap_envelope(&value);
    value
        .get("api_url")
        .or_else(|| value.get("apiUrl"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| format!("{CONFIG_RESOLVE_API_URL} returned no api_url"))
}

#[cfg(test)]
#[path = "link_tests.rs"]
mod tests;
