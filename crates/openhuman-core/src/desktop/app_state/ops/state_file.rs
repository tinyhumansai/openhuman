//! Local on-disk persistence for `StoredAppState`: an atomically-written
//! JSON file under the workspace `state/` directory, with corrupted files
//! quarantined rather than crashing the poll.

use super::types::StoredAppState;
use super::LOG_PREFIX;
use crate::config::Config;
use log::warn;
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use std::fs;
#[cfg(unix)]
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use tempfile::NamedTempFile;

// Deliberately local, not on the storage port: this file holds the local
// encryption key and the keyring consent choice, which decide how secrets are
// stored on this machine and are needed before any storage backend is open
// (bootstrap config comes from env or a file, never from storage). The
// onboarding flags beside them are tiny and per-install. Allowlisted in
// `scripts/ci/check-storage-bypass.mjs`.
pub(super) const APP_STATE_FILENAME: &str = "app-state.json";
pub(super) static APP_STATE_FILE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

pub(super) fn app_state_path(config: &Config) -> Result<PathBuf, String> {
    let state_dir = config.workspace_dir.join("state");
    fs::create_dir_all(&state_dir).map_err(|e| {
        format!(
            "failed to create workspace state dir {}: {e}",
            state_dir.display()
        )
    })?;
    Ok(state_dir.join(APP_STATE_FILENAME))
}

pub(super) fn corrupted_app_state_path(path: &Path) -> PathBuf {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or(0);
    path.with_extension(format!("json.corrupted.{timestamp}"))
}

pub(super) fn quarantine_corrupted_app_state(path: &Path, reason: &str) {
    let quarantine_path = corrupted_app_state_path(path);
    warn!(
        "{LOG_PREFIX} quarantining corrupted app state {} -> {} ({reason})",
        path.display(),
        quarantine_path.display()
    );

    if let Err(rename_error) = fs::rename(path, &quarantine_path) {
        warn!(
            "{LOG_PREFIX} failed to quarantine {} via rename: {}",
            path.display(),
            rename_error
        );
        if let Err(remove_error) = fs::remove_file(path) {
            warn!(
                "{LOG_PREFIX} failed to remove unreadable app state {}: {}",
                path.display(),
                remove_error
            );
        }
    }
}

pub(super) fn load_stored_app_state_unlocked(config: &Config) -> Result<StoredAppState, String> {
    let path = app_state_path(config)?;
    if !path.exists() {
        return Ok(StoredAppState::default());
    }

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(error) => {
            warn!(
                "{LOG_PREFIX} failed to read {}; falling back to defaults: {}",
                path.display(),
                error
            );
            quarantine_corrupted_app_state(&path, &error.to_string());
            return Ok(StoredAppState::default());
        }
    };

    match serde_json::from_str::<StoredAppState>(&raw) {
        Ok(state) => Ok(state),
        Err(error) => {
            warn!(
                "{LOG_PREFIX} failed to parse {}; falling back to defaults: {}",
                path.display(),
                error
            );
            quarantine_corrupted_app_state(&path, &error.to_string());
            Ok(StoredAppState::default())
        }
    }
}

pub(crate) fn load_stored_app_state(config: &Config) -> Result<StoredAppState, String> {
    let _guard = APP_STATE_FILE_LOCK.lock();
    load_stored_app_state_unlocked(config)
}

pub(super) fn sync_parent_dir(path: &Path) -> Result<(), String> {
    // Directory fsync is a POSIX-only durability guarantee — on Unix we
    // open the parent dir and call `sync_all()` so the rename of the
    // temp file into place is persisted even if the host crashes before
    // the next buffer flush. On Windows, opening a directory as a
    // regular file requires `FILE_FLAG_BACKUP_SEMANTICS` which
    // `std::fs::File::open` does not set, so the call fails with
    // "Access is denied. (os error 5)". Since Windows uses a different
    // durability model (and `NamedTempFile::persist` issues an atomic
    // MoveFileEx which is already durable enough for our config files),
    // we skip the fsync entirely on non-Unix and return Ok. Mirrors the
    // existing `sync_directory` guard in `config/schema/load.rs`.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("failed to sync directory {}: {e}", parent.display()))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

pub(super) fn save_stored_app_state_unlocked(
    config: &Config,
    state: &StoredAppState,
) -> Result<(), String> {
    let path = app_state_path(config)?;
    let payload = serde_json::to_string_pretty(state)
        .map_err(|e| format!("failed to serialize app state: {e}"))?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("failed to resolve parent dir for {}", path.display()))?;
    let mut temp_file = NamedTempFile::new_in(parent)
        .map_err(|e| format!("failed to create temp file in {}: {e}", parent.display()))?;
    temp_file
        .write_all(payload.as_bytes())
        .map_err(|e| format!("failed to write temp app state for {}: {e}", path.display()))?;
    temp_file
        .as_file_mut()
        .sync_all()
        .map_err(|e| format!("failed to sync temp app state for {}: {e}", path.display()))?;
    sync_parent_dir(&path)?;
    temp_file.persist(&path).map_err(|e| {
        format!(
            "failed to persist app state {}: {}",
            path.display(),
            e.error
        )
    })?;
    sync_parent_dir(&path)?;
    Ok(())
}

pub fn save_app_state(config: &Config, state: &StoredAppState) -> Result<(), String> {
    let _guard = APP_STATE_FILE_LOCK.lock();
    save_stored_app_state_unlocked(config, state)
}
