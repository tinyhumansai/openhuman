//! Per-source sync state: last run, status, error and item count.
//!
//! This is runtime state, not configuration, so it lives in
//! `<workspace>/memory/sources_state.json` rather than `config.toml`. A source
//! with no entry has never synced. A `syncing` status found on load belongs to
//! a run the previous process never finished, and reads as `idle`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::memory::types::SourceStatus;

/// Serialises state-file read-modify-writes in this process.
static LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

/// What the last sync of one source left behind.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceState {
    /// When the last sync finished.
    #[serde(default)]
    pub last_sync_at: Option<DateTime<Utc>>,
    /// Current status.
    #[serde(default)]
    pub status: SourceStatus,
    /// Why the last sync failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Items stored by the last successful sync.
    #[serde(default)]
    pub items: u64,
}

fn path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("memory").join("sources_state.json")
}

fn read_all(workspace_dir: &Path) -> BTreeMap<String, SourceState> {
    std::fs::read_to_string(path(workspace_dir))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_all(workspace_dir: &Path, all: &BTreeMap<String, SourceState>) {
    let file = path(workspace_dir);
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let json = serde_json::to_vec_pretty(all).map_err(std::io::Error::other)?;
            std::fs::write(&file, json)
        });
    if let Err(error) = result {
        tracing::warn!(error = %error, "[memory:sources] writing sync state failed");
    }
}

/// Every source's state.
#[must_use]
pub fn load(workspace_dir: &Path) -> BTreeMap<String, SourceState> {
    read_all(workspace_dir)
}

/// Applies `change` to `id`'s state and persists it.
pub fn update(workspace_dir: &Path, id: &str, change: impl FnOnce(&mut SourceState)) {
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut all = read_all(workspace_dir);
    change(all.entry(id.to_string()).or_default());
    write_all(workspace_dir, &all);
}

/// Drops `id`'s state.
pub fn remove(workspace_dir: &Path, id: &str) {
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut all = read_all(workspace_dir);
    if all.remove(id).is_some() {
        write_all(workspace_dir, &all);
    }
}

/// Marks every `syncing` source idle: called once at startup, when no sync
/// can be running yet.
pub fn reset_interrupted(workspace_dir: &Path) {
    let _guard = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut all = read_all(workspace_dir);
    let mut changed = false;
    for state in all.values_mut() {
        if state.status == SourceStatus::Syncing {
            state.status = SourceStatus::Idle;
            changed = true;
        }
    }
    if changed {
        write_all(workspace_dir, &all);
    }
}
