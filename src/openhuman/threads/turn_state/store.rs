//! Filesystem-backed snapshot store for [`super::types::TurnState`].
//!
//! One JSON file per thread under
//! `<workspace>/memory/conversations/turn_states/<hex(thread_id)>.json`.
//! Whole-file overwrite (latest snapshot wins). The presence of a file
//! means the turn was non-terminal at last write.
//!
//! Mutations are serialised through a single process-wide mutex so the
//! progress consumer cannot interleave a flush against an RPC handler
//! reading the same file.

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use log::{debug, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tempfile::NamedTempFile;

use super::types::{TurnLifecycle, TurnState};

const LOG_PREFIX: &str = "[threads:turn_state]";
const TURN_STATE_DIR: &str = "turn_states";
const SNAPSHOT_EXTENSION: &str = "json";
static TURN_STATE_LOCK: Lazy<Mutex<()>> = Lazy::new(|| Mutex::new(()));

/// Workspace-rooted handle that reads and writes per-thread turn snapshots.
#[derive(Debug, Clone)]
pub struct TurnStateStore {
    workspace_dir: PathBuf,
}

impl TurnStateStore {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }

    /// Atomically overwrite the snapshot for `state.thread_id`.
    pub fn put(&self, state: &TurnState) -> Result<(), String> {
        let _guard = TURN_STATE_LOCK.lock();
        let dir = self.ensure_dir()?;
        let path = self.snapshot_path(&state.thread_id);
        let mut tmp = NamedTempFile::new_in(&dir)
            .map_err(|e| format!("create turn-state tempfile in {}: {e}", dir.display()))?;
        let bytes =
            serde_json::to_vec_pretty(state).map_err(|e| format!("serialize turn state: {e}"))?;
        tmp.write_all(&bytes)
            .map_err(|e| format!("write turn-state tempfile: {e}"))?;
        tmp.as_file()
            .sync_all()
            .map_err(|e| format!("fsync turn-state tempfile: {e}"))?;
        tmp.persist(&path)
            .map_err(|e| format!("persist turn-state file {}: {e}", path.display()))?;
        // Sync the directory entry created by the rename — without
        // this a crash or power loss between persist() and the next
        // fs flush can drop the snapshot, defeating the cold-boot
        // recovery guarantee. Best-effort on platforms where opening
        // a directory for sync is not supported (Windows). The fsync
        // failure is logged but not fatal.
        if let Err(err) = sync_dir(&dir) {
            log::warn!("{LOG_PREFIX} failed to fsync {}: {err}", dir.display());
        }
        debug!(
            "{LOG_PREFIX} wrote snapshot thread={} lifecycle={:?} iter={}/{} timeline={}",
            state.thread_id,
            state.lifecycle,
            state.iteration,
            state.max_iterations,
            state.tool_timeline.len()
        );
        Ok(())
    }

    /// Return the snapshot for `thread_id`, or `None` if no file exists.
    pub fn get(&self, thread_id: &str) -> Result<Option<TurnState>, String> {
        let _guard = TURN_STATE_LOCK.lock();
        let path = self.snapshot_path(thread_id);
        if !path.exists() {
            return Ok(None);
        }
        read_snapshot(&path).map(Some)
    }

    /// Delete the snapshot for `thread_id`. Returns `true` if a file was
    /// removed, `false` if none existed.
    pub fn delete(&self, thread_id: &str) -> Result<bool, String> {
        let _guard = TURN_STATE_LOCK.lock();
        let path = self.snapshot_path(thread_id);
        if !path.exists() {
            return Ok(false);
        }
        fs::remove_file(&path)
            .map_err(|e| format!("remove turn-state file {}: {e}", path.display()))?;
        debug!("{LOG_PREFIX} deleted snapshot thread={}", thread_id);
        Ok(true)
    }

    /// List every persisted snapshot. Used by the UI to surface
    /// interrupted turns on cold boot.
    pub fn list(&self) -> Result<Vec<TurnState>, String> {
        let _guard = TURN_STATE_LOCK.lock();
        let dir = self.dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut snapshots = Vec::new();
        for entry in
            fs::read_dir(&dir).map_err(|e| format!("read turn-state dir {}: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| format!("read turn-state entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some(SNAPSHOT_EXTENSION) {
                continue;
            }
            match read_snapshot(&path) {
                Ok(snapshot) => snapshots.push(snapshot),
                Err(err) => warn!(
                    "{LOG_PREFIX} skip unreadable snapshot {}: {err}",
                    path.display()
                ),
            }
        }
        Ok(snapshots)
    }

    /// Remove every snapshot file in the turn-state directory,
    /// regardless of whether the contents are readable. Used by
    /// `threads_purge` to guarantee no stale or corrupted snapshot
    /// survives a destructive cleanup — `list()` only returns parseable
    /// snapshots, so iterating list+delete would silently leave
    /// half-written or schema-skewed files behind.
    ///
    /// Returns the count of files removed. Failures on individual
    /// entries propagate as the first error encountered (the rest of
    /// the directory is not touched once an error occurs, so a retry
    /// can pick up where this left off).
    pub fn clear_all(&self) -> Result<usize, String> {
        let _guard = TURN_STATE_LOCK.lock();
        let dir = self.dir();
        if !dir.exists() {
            return Ok(0);
        }
        let mut removed = 0usize;
        for entry in
            fs::read_dir(&dir).map_err(|e| format!("read turn-state dir {}: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| format!("read turn-state entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some(SNAPSHOT_EXTENSION) {
                continue;
            }
            fs::remove_file(&path)
                .map_err(|e| format!("remove turn-state file {}: {e}", path.display()))?;
            removed += 1;
        }
        if removed > 0 {
            debug!(
                "{LOG_PREFIX} cleared {removed} snapshots from {}",
                dir.display()
            );
        }
        Ok(removed)
    }

    /// Mark every persisted snapshot as `Interrupted`. Intended to be
    /// invoked from the web-channel provider on startup so the UI can
    /// distinguish stale turns left behind by a previous process from
    /// turns that are currently being driven in this session.
    pub fn mark_all_interrupted(&self, now_rfc3339: &str) -> Result<usize, String> {
        let snapshots = self.list()?;
        let mut count = 0usize;
        for mut snapshot in snapshots {
            if matches!(snapshot.lifecycle, TurnLifecycle::Interrupted) {
                continue;
            }
            snapshot.lifecycle = TurnLifecycle::Interrupted;
            snapshot.updated_at = now_rfc3339.to_string();
            snapshot.active_tool = None;
            snapshot.active_subagent = None;
            self.put(&snapshot)?;
            count += 1;
        }
        if count > 0 {
            debug!("{LOG_PREFIX} marked {count} snapshots as interrupted on startup");
        }
        Ok(count)
    }

    fn ensure_dir(&self) -> Result<PathBuf, String> {
        let dir = self.dir();
        fs::create_dir_all(&dir)
            .map_err(|e| format!("create turn-state dir {}: {e}", dir.display()))?;
        Ok(dir)
    }

    fn dir(&self) -> PathBuf {
        self.workspace_dir
            .join("memory")
            .join("conversations")
            .join(TURN_STATE_DIR)
    }

    fn snapshot_path(&self, thread_id: &str) -> PathBuf {
        self.dir().join(format!(
            "{}.{}",
            hex::encode(thread_id.as_bytes()),
            SNAPSHOT_EXTENSION
        ))
    }
}

/// Best-effort `fsync` of a directory entry. On Unix, opens the
/// directory for read and calls `sync_all` on the file handle. On
/// Windows this is a no-op — directory fsync is not exposed by the
/// platform and the rename's durability is provided by NTFS journaling.
#[cfg(unix)]
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    File::open(dir)?.sync_all()
}

#[cfg(not(unix))]
fn sync_dir(_dir: &Path) -> std::io::Result<()> {
    Ok(())
}

fn read_snapshot(path: &Path) -> Result<TurnState, String> {
    let mut file =
        File::open(path).map_err(|e| format!("open turn-state {}: {e}", path.display()))?;
    let mut buf = String::new();
    file.read_to_string(&mut buf)
        .map_err(|e| format!("read turn-state {}: {e}", path.display()))?;
    serde_json::from_str(&buf).map_err(|e| format!("parse turn-state {}: {e}", path.display()))
}

// Free-function wrappers mirroring `memory::conversations::store` so callers
// at the RPC layer don't have to instantiate `TurnStateStore` themselves.

pub fn put(workspace_dir: PathBuf, state: &TurnState) -> Result<(), String> {
    TurnStateStore::new(workspace_dir).put(state)
}

pub fn get(workspace_dir: PathBuf, thread_id: &str) -> Result<Option<TurnState>, String> {
    TurnStateStore::new(workspace_dir).get(thread_id)
}

pub fn delete(workspace_dir: PathBuf, thread_id: &str) -> Result<bool, String> {
    TurnStateStore::new(workspace_dir).delete(thread_id)
}

pub fn list(workspace_dir: PathBuf) -> Result<Vec<TurnState>, String> {
    TurnStateStore::new(workspace_dir).list()
}

pub fn clear_all(workspace_dir: PathBuf) -> Result<usize, String> {
    TurnStateStore::new(workspace_dir).clear_all()
}

pub fn mark_all_interrupted(workspace_dir: PathBuf, now_rfc3339: &str) -> Result<usize, String> {
    TurnStateStore::new(workspace_dir).mark_all_interrupted(now_rfc3339)
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
