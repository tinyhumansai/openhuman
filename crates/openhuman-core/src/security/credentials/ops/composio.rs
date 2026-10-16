//! Direct-mode (BYO API key) Composio credential storage.

use serde_json::json;

use crate::config::Config;
use crate::core::Outcome;
use crate::security::credentials::{AuthService, DEFAULT_AUTH_PROFILE_NAME};

/// Provider slot for the user-provided Composio API key when running in
/// direct mode (BYO key).
///
/// Parallel to [`crate::security::credentials::APP_SESSION_PROVIDER`] but
/// completely independent — the app-session JWT authenticates the user
/// against the managed backend, while this slot authenticates the user
/// against `backend.composio.dev`. Stored via the same
/// [`super::super::profiles::AuthProfilesStore`] backend (encrypted on disk
/// when `secrets.encrypt = true`).
pub const COMPOSIO_DIRECT_PROVIDER: &str = "composio-direct";

/// Persist the user-provided Composio API key to the encrypted credential
/// store under [`COMPOSIO_DIRECT_PROVIDER`].
///
/// **Never log the API key itself** — the debug line below records only
/// length and a length-of-stored marker. This honours the CLAUDE.md
/// debug-logging rule (`Never log secrets … redact or omit`).
pub async fn store_composio_api_key(
    config: &Config,
    api_key: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    let trimmed = api_key.trim();
    if trimmed.is_empty() {
        return Err("composio api_key must not be empty".to_string());
    }
    tracing::debug!(
        len = trimmed.len(),
        "[composio-direct] storing api key (redacted)"
    );
    let auth = AuthService::from_config(config);
    auth.store_provider_token(
        COMPOSIO_DIRECT_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        trimmed,
        std::collections::HashMap::new(),
        true,
    )
    .map_err(|e| e.to_string())?;

    Ok(Outcome::single_log(
        json!({ "stored": true, "provider": COMPOSIO_DIRECT_PROVIDER }),
        "composio direct api key stored",
    ))
}

/// Read the user-provided Composio API key from the encrypted credential
/// store. Returns `Ok(None)` when no key has been stored yet.
///
/// Used by [`crate::integrations::composio::client::resolve_composio_route`]
/// to decide whether direct mode can actually be activated.
pub fn get_composio_api_key(config: &Config) -> Result<Option<String>, String> {
    let auth = AuthService::from_config(config);
    let key = auth
        .get_provider_bearer_token(COMPOSIO_DIRECT_PROVIDER, None)
        .map_err(|e| e.to_string())?;
    Ok(key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()))
}

/// RPC wrapper around [`store_composio_api_key`] — accepts plain string
/// for symmetry with `store_provider_credentials` while only persisting
/// the trimmed value.
pub async fn rpc_store_composio_api_key(
    config: &Config,
    api_key: &str,
) -> Result<Outcome<serde_json::Value>, String> {
    store_composio_api_key(config, api_key).await
}

/// Remove the stored Composio direct-mode API key. Used when the user
/// switches back to backend mode and explicitly clears their key.
pub async fn clear_composio_api_key(config: &Config) -> Result<Outcome<serde_json::Value>, String> {
    tracing::debug!("[composio-direct] clearing stored api key");
    let auth = AuthService::from_config(config);
    let removed = auth
        .remove_profile(COMPOSIO_DIRECT_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(
        json!({ "removed": removed }),
        "composio direct api key cleared",
    ))
}
