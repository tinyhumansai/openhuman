//! Short-lived PKCE state storage for provider-owned OAuth flows.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::credentials::state_dir_from_config;
use openhuman_embed::__host::storage::secrets::{current, MemorySecrets, SecretStore};

const PENDING_SECRET: &str = "openai-oauth-pending";
const LEGACY_FILE: &str = "openai-oauth-pending.json";
const PENDING_TTL_SECS: u64 = 600;
const MAX_LEGACY_BYTES: u64 = 16 * 1024;

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;

static FALLBACK: LazyLock<Mutex<HashMap<PathBuf, Arc<MemorySecrets>>>> =
    LazyLock::new(Mutex::default);
static ADOPTED: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Mutex::default);

pub(super) fn store(config: &Config) -> Result<(PathBuf, Arc<dyn SecretStore>), String> {
    let state_dir = state_dir_from_config(config);
    let store: Arc<dyn SecretStore> =
        match current().map_err(|error| format!("open the OAuth pending store: {error}"))? {
            Some(secrets) => Arc::new(secrets),
            None => {
                let mut fallback = FALLBACK.lock().unwrap_or_else(PoisonError::into_inner);
                let memory = fallback
                    .entry(state_dir.clone())
                    .or_insert_with(|| Arc::new(MemorySecrets::new()));
                Arc::clone(memory) as Arc<dyn SecretStore>
            }
        };
    Ok((state_dir, store))
}

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
            tracing::debug!(error = %error, "legacy provider OAuth session unreadable");
            return;
        }
    };
    let Some(created_at) = serde_json::from_slice::<serde_json::Value>(&bytes)
        .ok()
        .and_then(|doc| doc.get("created_at").and_then(serde_json::Value::as_u64))
    else {
        return;
    };
    if now_secs().saturating_sub(created_at) > PENDING_TTL_SECS {
        return;
    }
    match store.get(PENDING_SECRET).await {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(error) => {
            tracing::debug!(error = %error, "pending provider OAuth state unreadable");
            return;
        }
    }
    if let Err(error) = store.set(PENDING_SECRET, &bytes).await {
        tracing::warn!(error = %error, "could not adopt legacy provider OAuth state");
    }
}

pub(super) async fn clear(config: &Config) -> Result<(), String> {
    let (state_dir, store) = store(config)?;
    store
        .delete(PENDING_SECRET)
        .await
        .map_err(|error| format!("clear pending provider OAuth state: {error}"))?;
    ADOPTED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(state_dir);
    Ok(())
}

async fn read_bounded(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let meta = match tokio::fs::metadata(path).await {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if meta.len() > MAX_LEGACY_BYTES {
        return Err(std::io::Error::other("legacy OAuth state is oversized"));
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
}
