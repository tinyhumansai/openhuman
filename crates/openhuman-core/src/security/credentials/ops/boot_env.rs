//! Boot-time credential seeding for hosts with no interactive login: a
//! Docker / VPS core, a CI sidecar, an operator's shell. The credential still
//! has to be obtained elsewhere (the TinyHumans dashboard, a shell login);
//! this only installs it.
//!
//! * `OPENHUMAN_BACKEND_API_KEY` — a TinyHumans API key.
//! * `OPENHUMAN_BACKEND_SESSION_TOKEN` — a session JWT (must carry a subject
//!   claim, since no host is around to supply the user id).
//!
//! A credential already in the store is never overwritten: the env var seeds
//! a fresh install and is otherwise ignored, so rotating it means clearing the
//! stored one first (`openhuman-core auth clear_credential`).

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::security::credentials::api_key;
use crate::security::credentials::session_support::{get_session_token, CredentialKind};

use super::credential::{set_credential, SetCredentialRequest};

pub const BACKEND_API_KEY_ENV: &str = "OPENHUMAN_BACKEND_API_KEY";
pub const BACKEND_SESSION_TOKEN_ENV: &str = "OPENHUMAN_BACKEND_SESSION_TOKEN";

fn env_secret(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Install the API key from `OPENHUMAN_BACKEND_API_KEY` when none is stored.
/// Synchronous (a plain profile write) so it can run before the scheduler
/// gate is seeded from the store.
pub fn seed_api_key_from_env(config: &Config) {
    let Some(key) = env_secret(BACKEND_API_KEY_ENV) else {
        return;
    };
    if api_key::has_api_key(config) {
        log::debug!("[auth][boot] {BACKEND_API_KEY_ENV} set but an api key is already stored; keeping the stored one");
        return;
    }
    match api_key::store_api_key(config, &key) {
        Ok(_) => log::info!("[auth][boot] api key installed from {BACKEND_API_KEY_ENV}"),
        Err(error) => {
            log::warn!("[auth][boot] failed to install api key from {BACKEND_API_KEY_ENV}: {error}")
        }
    }
}

/// Install the session JWT from `OPENHUMAN_BACKEND_SESSION_TOKEN` when no
/// session is stored. Runs the full `set_credential` path (user-dir
/// activation, gated services, scheduler gate), so it is async.
pub async fn seed_session_from_env(ctx: &DispatchCtx, config: &Config) {
    let Some(token) = env_secret(BACKEND_SESSION_TOKEN_ENV) else {
        return;
    };
    if matches!(get_session_token(config), Ok(Some(_))) {
        log::debug!("[auth][boot] {BACKEND_SESSION_TOKEN_ENV} set but a session is already stored; keeping the stored one");
        return;
    }
    let request = SetCredentialRequest {
        token,
        kind: Some(CredentialKind::Session.as_str().to_string()),
        user_id: None,
        user: None,
        issuing_backend: None,
    };
    match set_credential(ctx, config, request).await {
        Ok(outcome) => log::info!(
            "[auth][boot] session installed from {BACKEND_SESSION_TOKEN_ENV} (user_id={})",
            outcome.value.user_id.as_deref().unwrap_or("?")
        ),
        Err(error) => log::warn!(
            "[auth][boot] failed to install session from {BACKEND_SESSION_TOKEN_ENV}: {error}"
        ),
    }
}

#[cfg(test)]
#[path = "boot_env_tests.rs"]
mod tests;
