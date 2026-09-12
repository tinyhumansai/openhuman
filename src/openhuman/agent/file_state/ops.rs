//! Operational API for the file state coordinator.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Instant, SystemTime};
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::types::{FileStateCoordinator, ReadStamp, WriteStamp};

// ── Singleton ────────────────────────────────────────────────────────────

static GLOBAL: OnceLock<Arc<FileStateCoordinator>> = OnceLock::new();

/// Returns `true` when the guard is disabled via env var.
fn is_disabled() -> bool {
    static DISABLED: OnceLock<bool> = OnceLock::new();
    *DISABLED.get_or_init(|| {
        std::env::var("OPENHUMAN_FILE_STATE_GUARD")
            .map(|v| matches!(v.as_str(), "0" | "false" | "off" | "no"))
            .unwrap_or(false)
    })
}

/// Initialise the process-global coordinator. Safe to call multiple times;
/// only the first call wins.
pub fn init_global() {
    if is_disabled() {
        tracing::debug!("[file_state] guard disabled via OPENHUMAN_FILE_STATE_GUARD");
        return;
    }
    let _ = GLOBAL.set(Arc::new(FileStateCoordinator::new()));
    tracing::debug!("[file_state] coordinator initialised");
}

/// Returns the global coordinator, or `None` when disabled / not yet initialised.
pub fn try_global() -> Option<Arc<FileStateCoordinator>> {
    if is_disabled() {
        return None;
    }
    GLOBAL.get().cloned()
}

// ── Read tracking ────────────────────────────────────────────────────────

/// Record that `agent_id` read `resolved_path` at the given mtime.
pub fn record_read(agent_id: &str, resolved_path: PathBuf, mtime: SystemTime, partial: bool) {
    let Some(coord) = try_global() else { return };
    tracing::trace!(
        agent = agent_id,
        path = %resolved_path.display(),
        partial,
        "[file_state] record_read"
    );
    coord.reads.write().insert(
        (agent_id.to_string(), resolved_path),
        ReadStamp {
            mtime,
            timestamp: Instant::now(),
            partial,
        },
    );
}

// ── Write tracking ───────────────────────────────────────────────────────

/// Record that `agent_id` wrote `resolved_path`.
pub fn record_write(agent_id: &str, resolved_path: PathBuf) {
    let Some(coord) = try_global() else { return };
    tracing::trace!(
        agent = agent_id,
        path = %resolved_path.display(),
        "[file_state] record_write"
    );
    let now = Instant::now();
    coord.writes.write().insert(
        resolved_path.clone(),
        WriteStamp {
            writer: agent_id.to_string(),
            timestamp: now,
        },
    );
    // Also update this agent's own read stamp so its own subsequent
    // writes don't trigger self-staleness.
    coord.reads.write().insert(
        (agent_id.to_string(), resolved_path),
        ReadStamp {
            mtime: SystemTime::now(),
            timestamp: now,
            partial: false,
        },
    );
}

// ── Staleness checks ─────────────────────────────────────────────────────

/// Check whether `agent_id`'s view of `resolved_path` is stale because
/// another agent wrote to it after this agent's last read. Returns an
/// error message when stale, `None` when safe.
pub fn check_stale_read(agent_id: &str, resolved_path: &PathBuf) -> Option<String> {
    let coord = try_global()?;
    let reads = coord.reads.read();
    let writes = coord.writes.read();
    let read_key = (agent_id.to_string(), resolved_path.clone());
    let read_stamp = reads.get(&read_key)?;
    let ws = writes.get(resolved_path)?;
    if ws.writer != agent_id && ws.timestamp > read_stamp.timestamp {
        let display_path = resolved_path.display();
        Some(format!(
            "Stale read: file '{display_path}' was modified by agent '{}' after your last read. \
             Re-read the file before editing.",
            ws.writer
        ))
    } else {
        None
    }
}

/// Check whether `agent_id`'s last read of `resolved_path` was partial.
/// Returns an error message when partial, `None` when safe.
pub fn check_partial_read(agent_id: &str, resolved_path: &Path) -> Option<String> {
    let coord = try_global()?;
    let reads = coord.reads.read();
    let read_key = (agent_id.to_string(), resolved_path.to_path_buf());
    let read_stamp = reads.get(&read_key)?;
    if read_stamp.partial {
        let display_path = resolved_path.display();
        Some(format!(
            "Partial read: your last read of '{display_path}' was partial (paginated). \
             Perform a full read before overwriting."
        ))
    } else {
        None
    }
}

// ── Path locking ─────────────────────────────────────────────────────────

/// Acquire an async lock on `resolved_path` for a read-modify-write
/// section. Returns an `OwnedMutexGuard` that releases when dropped.
/// Returns `None` when the coordinator is disabled.
pub async fn acquire_path_lock(resolved_path: &Path) -> Option<OwnedMutexGuard<()>> {
    let coord = try_global()?;
    let mutex = {
        let mut locks = coord.path_locks.write();
        locks
            .entry(resolved_path.to_path_buf())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };
    Some(mutex.lock_owned().await)
}

// ── Parent reminder ──────────────────────────────────────────────────────

/// Return resolved paths that `parent_agent_id` had previously read but
/// were subsequently written by any agent in `child_agent_ids`.
pub fn parent_stale_files(parent_agent_id: &str, child_agent_ids: &[String]) -> Vec<PathBuf> {
    let Some(coord) = try_global() else {
        return Vec::new();
    };
    let reads = coord.reads.read();
    let writes = coord.writes.read();
    let mut stale = Vec::new();
    for ((agent_id, path), read_stamp) in reads.iter() {
        if agent_id != parent_agent_id {
            continue;
        }
        if let Some(ws) = writes.get(path) {
            if child_agent_ids.contains(&ws.writer) && ws.timestamp > read_stamp.timestamp {
                stale.push(path.clone());
            }
        }
    }
    stale.sort();
    stale.dedup();
    stale
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
