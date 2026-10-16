//! The TinyHumans **API key** credential — the only credential a library
//! embedder holds.
//!
//! Library mode has no user login: no `/auth/me` round trip, no session JWT,
//! no `exp` to track. The host hands the core one API key at runtime build
//! time and every managed-backend request authenticates with it:
//!
//! * managed inference (`{api_url}` + the inference path) sends it as
//!   `Authorization: Bearer <key>` — the OpenAI-compatible endpoint takes a
//!   bearer, and `OpenHumanBackendModel::resolve_bearer` prefers this profile
//!   over the app session;
//! * SDK REST clients send it as `x-api-key` — `BackendClient` and
//!   `IntegrationClient` pick the header from [`BackendCredential`];
//! * bearer-only callers (cloud STT and embeddings, the connector proxy
//!   route, the memory host) send it as
//!   `Authorization: Bearer <key>`, which the backend accepts by prefix.
//!
//! The key lives in the same auth-profile store as the app session (the
//! parent of `config.config_path`), under its own provider id, so a runtime
//! that has both prefers the API key (see
//! [`resolve_backend_credential`](super::session_support::resolve_backend_credential)).
//! It is never written to `config.toml` and never logged.

use std::collections::HashMap;
use std::path::Path;

use anyhow::Result;

use super::core::{AuthService, DEFAULT_AUTH_PROFILE_NAME};
use super::profiles::AuthProfile;
use crate::config::Config;

/// Provider id of the API-key auth profile.
pub const API_KEY_PROVIDER: &str = "api-key";

/// Metadata marker distinguishing the profile from any other token profile.
pub const API_KEY_KIND_META: &str = "kind";
/// Value of [`API_KEY_KIND_META`].
pub const API_KEY_KIND: &str = "tinyhumans-api-key";

/// Persist `key` as the active API-key profile beside `config`'s other
/// credentials. Blank keys are rejected rather than stored.
pub fn store_api_key(config: &Config, key: &str) -> Result<AuthProfile> {
    store_api_key_in(
        &super::core::state_dir_from_config(config),
        config.secrets.encrypt,
        key,
    )
}

/// [`store_api_key`] against an explicit credential state directory. Used by
/// a library runtime that must install the key *before* the core boots (the
/// scheduler gate reads the credential store once, at boot).
pub fn store_api_key_in(state_dir: &Path, encrypt: bool, key: &str) -> Result<AuthProfile> {
    let key = key.trim();
    if key.is_empty() {
        anyhow::bail!("API key is blank");
    }
    let mut metadata = HashMap::new();
    metadata.insert(API_KEY_KIND_META.to_string(), API_KEY_KIND.to_string());
    log::debug!(
        "[credentials][api-key] storing api-key profile state_dir={}",
        state_dir.display()
    );
    AuthService::new(state_dir, encrypt).store_provider_token(
        API_KEY_PROVIDER,
        DEFAULT_AUTH_PROFILE_NAME,
        key,
        metadata,
        true,
    )
}

/// The stored API key, if any.
pub fn get_api_key(config: &Config) -> Result<Option<String>> {
    get_api_key_in(
        &super::core::state_dir_from_config(config),
        config.secrets.encrypt,
    )
}

/// [`get_api_key`] against an explicit credential state directory — for
/// callers that hold `ProviderRuntimeOptions` rather than a `Config`.
///
/// Requires the [`API_KEY_KIND_META`] marker, not just a profile named
/// [`API_KEY_PROVIDER`]. The generic `auth_store_provider_credentials`
/// RPC and CLI let a caller store an ordinary provider profile under any
/// name, `"api-key"` included; without the marker check, such a profile
/// would be accepted here as a TinyHumans runtime key and its bearer sent to
/// the managed backend as one.
pub fn get_api_key_in(state_dir: &Path, encrypt: bool) -> Result<Option<String>> {
    let service = AuthService::new(state_dir, encrypt);
    let Some(profile) = service.get_profile(API_KEY_PROVIDER, None)? else {
        return Ok(None);
    };
    if profile.metadata.get(API_KEY_KIND_META).map(String::as_str) != Some(API_KEY_KIND) {
        log::debug!(
            "[credentials][api-key] provider profile {:?} exists but lacks the {API_KEY_KIND_META}={API_KEY_KIND} \
             marker — treating as absent, not a TinyHumans api key",
            API_KEY_PROVIDER
        );
        return Ok(None);
    }
    let credential = match profile.kind {
        super::profiles::AuthProfileKind::Token => profile.token,
        super::profiles::AuthProfileKind::OAuth => profile.token_set.map(|t| t.access_token),
    };
    Ok(credential.filter(|t| !t.trim().is_empty()))
}

/// Whether an API key is stored. Errors reading the store count as "no key"
/// so a corrupt profile file degrades to the session path instead of
/// panicking a hot path; the error is logged.
pub fn has_api_key_in(state_dir: &Path, encrypt: bool) -> bool {
    match get_api_key_in(state_dir, encrypt) {
        Ok(key) => key.is_some(),
        Err(e) => {
            log::debug!("[credentials][api-key] store read failed, treating as absent: {e:#}");
            false
        }
    }
}

/// Whether an API key is stored for `config`.
pub fn has_api_key(config: &Config) -> bool {
    has_api_key_in(
        &super::core::state_dir_from_config(config),
        config.secrets.encrypt,
    )
}

/// Remove the API-key profile. Returns whether one existed.
pub fn clear_api_key(config: &Config) -> Result<bool> {
    log::debug!("[credentials][api-key] clearing api-key profile");
    AuthService::from_config(config).remove_profile(API_KEY_PROVIDER, DEFAULT_AUTH_PROFILE_NAME)
}

#[cfg(test)]
#[path = "api_key_tests.rs"]
mod tests;
