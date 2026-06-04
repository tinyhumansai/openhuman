//! Operational API for the file state coordinator.

use std::path::PathBuf;
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
pub fn check_partial_read(agent_id: &str, resolved_path: &PathBuf) -> Option<String> {
    let coord = try_global()?;
    let reads = coord.reads.read();
    let read_key = (agent_id.to_string(), resolved_path.clone());
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
pub async fn acquire_path_lock(resolved_path: &PathBuf) -> Option<OwnedMutexGuard<()>> {
    let coord = try_global()?;
    let mutex = {
        let mut locks = coord.path_locks.write();
        locks
            .entry(resolved_path.clone())
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
mod tests {
    use super::*;
    use std::time::Duration;

    fn fresh_coordinator() -> Arc<FileStateCoordinator> {
        Arc::new(FileStateCoordinator::new())
    }

    #[test]
    fn record_and_check_no_staleness() {
        let coord = fresh_coordinator();
        let path = PathBuf::from("/tmp/test/a.txt");
        coord.reads.write().insert(
            ("agent-a".to_string(), path.clone()),
            ReadStamp {
                mtime: SystemTime::now(),
                timestamp: Instant::now(),
                partial: false,
            },
        );
        let reads = coord.reads.read();
        let rs = reads.get(&("agent-a".to_string(), path.clone())).unwrap();
        assert!(!rs.partial);
        assert!(coord.writes.read().get(&path).is_none());
    }

    #[test]
    fn detect_sibling_write_staleness() {
        let coord = fresh_coordinator();
        let path = PathBuf::from("/tmp/test/b.txt");
        let read_time = Instant::now();
        coord.reads.write().insert(
            ("agent-a".to_string(), path.clone()),
            ReadStamp {
                mtime: SystemTime::now(),
                timestamp: read_time,
                partial: false,
            },
        );
        std::thread::sleep(Duration::from_millis(5));
        coord.writes.write().insert(
            path.clone(),
            WriteStamp {
                writer: "agent-b".to_string(),
                timestamp: Instant::now(),
            },
        );
        let stale = coord.stale_reads_for_parent("agent-a");
        assert_eq!(stale, vec![path]);
    }

    #[test]
    fn own_write_does_not_trigger_staleness() {
        let coord = fresh_coordinator();
        let path = PathBuf::from("/tmp/test/c.txt");
        let now = Instant::now();
        coord.reads.write().insert(
            ("agent-a".to_string(), path.clone()),
            ReadStamp {
                mtime: SystemTime::now(),
                timestamp: now,
                partial: false,
            },
        );
        std::thread::sleep(Duration::from_millis(5));
        coord.writes.write().insert(
            path.clone(),
            WriteStamp {
                writer: "agent-a".to_string(),
                timestamp: Instant::now(),
            },
        );
        let stale = coord.stale_reads_for_parent("agent-a");
        assert!(stale.is_empty());
    }

    #[test]
    fn partial_read_detected() {
        let coord = fresh_coordinator();
        let path = PathBuf::from("/tmp/test/d.txt");
        coord.reads.write().insert(
            ("agent-a".to_string(), path.clone()),
            ReadStamp {
                mtime: SystemTime::now(),
                timestamp: Instant::now(),
                partial: true,
            },
        );
        let reads = coord.reads.read();
        let rs = reads.get(&("agent-a".to_string(), path.clone())).unwrap();
        assert!(rs.partial);
    }

    #[test]
    fn parent_stale_files_detects_child_writes() {
        let coord = fresh_coordinator();
        let path = PathBuf::from("/tmp/test/e.txt");
        let parent_read_time = Instant::now();
        coord.reads.write().insert(
            ("parent".to_string(), path.clone()),
            ReadStamp {
                mtime: SystemTime::now(),
                timestamp: parent_read_time,
                partial: false,
            },
        );
        std::thread::sleep(Duration::from_millis(5));
        coord.writes.write().insert(
            path.clone(),
            WriteStamp {
                writer: "child-1".to_string(),
                timestamp: Instant::now(),
            },
        );
        let stale = coord.stale_reads_for_parent("parent");
        assert_eq!(stale, vec![path]);
    }

    #[test]
    fn paths_written_by_collects_correctly() {
        let coord = fresh_coordinator();
        let p1 = PathBuf::from("/tmp/test/f1.txt");
        let p2 = PathBuf::from("/tmp/test/f2.txt");
        coord.writes.write().insert(
            p1.clone(),
            WriteStamp {
                writer: "child-1".to_string(),
                timestamp: Instant::now(),
            },
        );
        coord.writes.write().insert(
            p2.clone(),
            WriteStamp {
                writer: "child-2".to_string(),
                timestamp: Instant::now(),
            },
        );
        let result = coord.paths_written_by(&["child-1".to_string()]);
        assert_eq!(result.len(), 1);
        assert!(result.contains_key("child-1"));
        assert_eq!(result["child-1"], vec![p1]);
    }

    #[tokio::test]
    async fn path_lock_serialises_access() {
        let coord = fresh_coordinator();
        let path = PathBuf::from("/tmp/test/lock.txt");
        let mutex = {
            let mut locks = coord.path_locks.write();
            locks
                .entry(path.clone())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };

        let guard = mutex.lock().await;
        assert!(mutex.try_lock().is_err());
        drop(guard);
        assert!(mutex.try_lock().is_ok());
    }
}
