//! Global in-memory registry for vault sync progress state.
//!
//! State is keyed by `vault_id` and lives for the lifetime of the process.
//! A completed/failed entry is retained until the next sync for the same
//! vault overwrites it, so the frontend can always read the last outcome.
//!
//! Uses `once_cell::sync::Lazy` + `parking_lot::RwLock` — no heap allocation
//! at import time, no `std::sync::Mutex` poisoning risk.

use std::collections::HashMap;

use once_cell::sync::Lazy;
use parking_lot::RwLock;

use super::types::{VaultSyncState, VaultSyncStatus};

static SYNC_STATE: Lazy<RwLock<HashMap<String, VaultSyncState>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// Return the current sync state for a vault, or `None` if no sync has run.
pub fn get(vault_id: &str) -> Option<VaultSyncState> {
    SYNC_STATE.read().get(vault_id).cloned()
}

/// Replace the sync state for a vault (creates or overwrites the entry).
pub fn set(state: VaultSyncState) {
    SYNC_STATE.write().insert(state.vault_id.clone(), state);
}

/// Transition a vault to `Running`.
///
/// Returns `Err` if a sync for this vault is already in progress so the
/// caller can reject duplicate requests.
pub fn start(vault_id: &str, started_at_ms: i64) -> Result<(), String> {
    let mut map = SYNC_STATE.write();
    if let Some(s) = map.get(vault_id) {
        if s.status == VaultSyncStatus::Running {
            log::debug!(
                "[vault][state] start rejected: vault_id={vault_id} already running since={}",
                s.started_at_ms
            );
            return Err(format!("vault {vault_id} is already syncing"));
        }
    }
    log::debug!("[vault][state] start: vault_id={vault_id} started_at_ms={started_at_ms}");
    map.insert(
        vault_id.to_string(),
        VaultSyncState {
            vault_id: vault_id.to_string(),
            status: VaultSyncStatus::Running,
            scanned: 0,
            ingested: 0,
            unchanged: 0,
            removed: 0,
            failed: 0,
            skipped_unsupported: 0,
            total: 0,
            started_at_ms,
            finished_at_ms: None,
            duration_ms: 0,
            errors: vec![],
        },
    );
    Ok(())
}

/// Apply `f` to the current `VaultSyncState` for `vault_id` in-place.
///
/// No-ops silently if no entry exists (e.g. if the registry was cleared or
/// the vault_id is wrong — neither should happen in normal operation).
pub fn update_progress(vault_id: &str, f: impl FnOnce(&mut VaultSyncState)) {
    let mut map = SYNC_STATE.write();
    if let Some(s) = map.get_mut(vault_id) {
        f(s);
    } else {
        log::debug!(
            "[vault][state] update_progress no-op: vault_id={vault_id} not found in state map"
        );
    }
}
