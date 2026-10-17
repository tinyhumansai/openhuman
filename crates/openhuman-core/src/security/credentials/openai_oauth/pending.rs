//! Where the OAuth flow keeps its short-lived pending PKCE session.
//!
//! TinyInference's `OAuthFlow` holds the pending session (the PKCE verifier is
//! a bearer-equivalent value) in a `SecretStore` the host hands it, so the host
//! picks the place:
//!
//! * with a storage backend installed, the acting scope's encrypted secrets
//!   ([`crate::storage::secrets::current`]), the same place the credential
//!   stores keep their data;
//! * with none (a unit test, an embedder that skipped the host boot), process
//!   memory, one store per state directory. The session lives ten minutes and
//!   is completed by the process that started it, so nothing durable is lost.
//!
//! Before the secret store the session was `<state dir>/openai-oauth-pending.json`.
//! A session still inside its ten minutes at upgrade is adopted once per
//! process ([`adopt_legacy`]); the file is read and never modified or deleted,
//! and a stale one is ignored.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use tinystoragedrivers::secrets::{MemorySecrets, SecretStore};

use crate::config::Config;
use crate::security::credentials::state_dir_from_config;

const LOG_PREFIX: &str = "[inference][openai-oauth]";
/// The secret name TinyInference's `openai_codex_config` stores the session
/// under.
const PENDING_SECRET: &str = "openai-oauth-pending";
/// The file the session lived in before the secret store.
const LEGACY_FILE: &str = "openai-oauth-pending.json";
/// How long a pending session is valid (TinyInference's own limit).
const PENDING_TTL_SECS: u64 = 600;
/// A legacy session file is a few hundred bytes; anything bigger is not one.
const MAX_LEGACY_BYTES: u64 = 16 * 1024;

/// Memory-backed stores for hosts with no backend installed, by state dir.
static FALLBACK: LazyLock<Mutex<HashMap<PathBuf, Arc<MemorySecrets>>>> =
    LazyLock::new(Mutex::default);

/// State dirs whose legacy file this process already looked at.
static ADOPTED: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Mutex::default);

/// The secret store the pending session lives in for `config`'s current call.
///
/// # Errors
///
/// When a backend is installed but its scope or master key cannot be had.
pub(super) fn store(config: &Config) -> Result<Arc<dyn SecretStore>, String> {
    let state_dir = state_dir_from_config(config);
    let store: Arc<dyn SecretStore> = match crate::storage::secrets::current()
        .map_err(|error| format!("open the OAuth pending store: {error}"))?
    {
        Some(secrets) => Arc::new(secrets),
        None => {
            let mut fallback = FALLBACK.lock().unwrap_or_else(PoisonError::into_inner);
            let memory = fallback
                .entry(state_dir.clone())
                .or_insert_with(|| Arc::new(MemorySecrets::new()));
            Arc::clone(memory) as Arc<dyn SecretStore>
        }
    };
    Ok(store)
}

/// Adopts a legacy pending session file into `store`, once per process and
/// state directory. A session already in the store wins, and an expired or
/// unreadable file is ignored. Never fails the flow.
pub(super) async fn adopt_legacy(state_dir: &Path, store: &Arc<dyn SecretStore>) {
    {
        let mut seen = ADOPTED.lock().unwrap_or_else(PoisonError::into_inner);
        if !seen.insert(state_dir.to_path_buf()) {
            return;
        }
    }
    let path = state_dir.join(LEGACY_FILE);
    let bytes = match read_bounded(&path).await {
        Ok(Some(bytes)) => bytes,
        Ok(None) => return,
        Err(error) => {
            tracing::debug!(error = %error, "{LOG_PREFIX} legacy pending session unreadable");
            return;
        }
    };
    let Some(created_at) = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|doc| doc.get("created_at").and_then(serde_json::Value::as_u64))
    else {
        tracing::debug!("{LOG_PREFIX} legacy pending session has no creation time; ignored");
        return;
    };
    if now_secs().saturating_sub(created_at) > PENDING_TTL_SECS {
        tracing::debug!("{LOG_PREFIX} legacy pending session expired; ignored");
        return;
    }
    match store.get(PENDING_SECRET).await {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(error = %error, "{LOG_PREFIX} pending store unreadable; legacy session skipped");
            return;
        }
    }
    match store.set(PENDING_SECRET, &bytes).await {
        Ok(()) => tracing::info!("{LOG_PREFIX} adopted the legacy pending session"),
        Err(error) => {
            tracing::warn!(error = %error, "{LOG_PREFIX} could not adopt the legacy pending session");
        }
    }
}

async fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let meta = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if meta.len() > MAX_LEGACY_BYTES {
        return Err(std::io::Error::other("legacy pending session is oversized"));
    }
    tokio::fs::read(path).await.map(Some)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
pub(super) fn forget_for_tests(state_dir: &Path) {
    ADOPTED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(state_dir);
    FALLBACK
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(state_dir);
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
