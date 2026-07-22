//! Filesystem-backed snapshot store for [`super::types::TurnState`].
//!
//! **Per-turn ring layout.** One JSON file per *turn* under a per-thread
//! directory:
//! `<workspace>/memory/conversations/turn_states/<hex(thread_id)>/<hex(request_id)>.json`.
//! Each turn keeps its own snapshot so a multi-turn thread retains every turn's
//! tool timeline (the "Agentic task insights" trail), not just the latest.
//! Completed turns are pruned to the newest [`COMPLETED_RETENTION`] per thread so
//! history stays bounded.
//!
//! The `get(thread_id)` / `list()` / `delete(thread_id)` / `clear_all` /
//! `mark_all_interrupted` surface is unchanged so existing callers (RPC layer,
//! mirror, cold-boot) keep working: `get`/`list` resolve the *latest* turn per
//! thread. New `get_turn(thread_id, request_id)` and `list_thread(thread_id)`
//! expose the per-turn history.
//!
//! **Legacy migration.** Snapshots written by older cores live as flat files
//! `turn_states/<hex(thread_id)>.json`. They are migrated in place — read once,
//! rewritten under `<hex(thread_id)>/<hex(request_id)>.json`, and the flat file
//! removed — on first access. Migration is idempotent.
//!
//! Mutations are serialised through a single process-wide mutex so the progress
//! consumer cannot interleave a flush against an RPC handler reading the same
//! file.

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
/// Newest completed turns retained per thread. Older completed turns are pruned
/// on the next completed write so a long-lived thread's history stays bounded
/// (mirrors the timeline registry's soft-cap philosophy — never unbounded).
const COMPLETED_RETENTION: usize = 20;
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

    /// Workspace root this store persists under. Exposed so the mirror can
    /// resolve sibling session transcripts (append the interrupted partial to
    /// `session_raw/{root}.jsonl`) without re-plumbing the path.
    pub fn workspace_dir(&self) -> &std::path::Path {
        &self.workspace_dir
    }

    /// Atomically write the snapshot for `state.request_id` under
    /// `state.thread_id`. On a `Completed` write, prune the thread's completed
    /// turns to the newest [`COMPLETED_RETENTION`].
    pub fn put(&self, state: &TurnState) -> Result<(), String> {
        let _guard = TURN_STATE_LOCK.lock();
        // Fold any pre-existing flat file for this thread into the per-turn
        // layout first so the directory is the single source of truth.
        self.migrate_thread_locked(&state.thread_id);
        let dir = self.ensure_thread_dir(&state.thread_id)?;
        let path = self.turn_path(&state.thread_id, &state.request_id);
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
        // Sync the directory entry created by the rename — without this a crash
        // or power loss between persist() and the next fs flush can drop the
        // snapshot, defeating the cold-boot recovery guarantee. Best-effort on
        // platforms where opening a directory for sync is not supported.
        if let Err(err) = sync_dir(&dir) {
            log::warn!("{LOG_PREFIX} failed to fsync {}: {err}", dir.display());
        }
        debug!(
            "{LOG_PREFIX} wrote snapshot thread={} request={} lifecycle={:?} iter={}/{} timeline={}",
            state.thread_id,
            state.request_id,
            state.lifecycle,
            state.iteration,
            state.max_iterations,
            state.tool_timeline.len()
        );
        if state.lifecycle == TurnLifecycle::Completed {
            self.prune_completed_locked(&state.thread_id);
        }
        Ok(())
    }

    /// Return the latest turn for `thread_id`, or `None` if none exists.
    /// "Latest" is the turn with the greatest `started_at` (ties broken by
    /// `updated_at`) — the in-flight or most-recent turn.
    pub fn get(&self, thread_id: &str) -> Result<Option<TurnState>, String> {
        let _guard = TURN_STATE_LOCK.lock();
        self.migrate_thread_locked(thread_id);
        Ok(latest_turn(self.read_thread_turns(thread_id)?))
    }

    /// Return a specific turn by `request_id`, or `None` if absent.
    pub fn get_turn(&self, thread_id: &str, request_id: &str) -> Result<Option<TurnState>, String> {
        let _guard = TURN_STATE_LOCK.lock();
        self.migrate_thread_locked(thread_id);
        let path = self.turn_path(thread_id, request_id);
        if !path.exists() {
            return Ok(None);
        }
        read_snapshot(&path).map(Some)
    }

    /// Delete every turn for `thread_id` (and any legacy flat file). Returns
    /// `true` if anything was removed.
    pub fn delete(&self, thread_id: &str) -> Result<bool, String> {
        let _guard = TURN_STATE_LOCK.lock();
        let mut removed = false;
        let flat = self.legacy_flat_path(thread_id);
        if flat.exists() {
            fs::remove_file(&flat)
                .map_err(|e| format!("remove legacy turn-state {}: {e}", flat.display()))?;
            removed = true;
        }
        let dir = self.thread_dir(thread_id);
        if dir.exists() {
            fs::remove_dir_all(&dir)
                .map_err(|e| format!("remove turn-state dir {}: {e}", dir.display()))?;
            removed = true;
        }
        if removed {
            debug!("{LOG_PREFIX} deleted snapshots thread={}", thread_id);
        }
        Ok(removed)
    }

    /// List the latest turn for every thread. Used by the UI on cold boot to
    /// surface interrupted turns from a previous process (one entry per thread,
    /// preserving the pre-ring-store contract).
    pub fn list(&self) -> Result<Vec<TurnState>, String> {
        let _guard = TURN_STATE_LOCK.lock();
        self.migrate_all_legacy_locked();
        let dir = self.dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut snapshots = Vec::new();
        for thread_id in self.thread_ids()? {
            if let Some(latest) = latest_turn(self.read_thread_turns(&thread_id)?) {
                snapshots.push(latest);
            }
        }
        Ok(snapshots)
    }

    /// List every turn for one thread, newest first (by `started_at`).
    pub fn list_thread(&self, thread_id: &str) -> Result<Vec<TurnState>, String> {
        let _guard = TURN_STATE_LOCK.lock();
        self.migrate_thread_locked(thread_id);
        let mut turns = self.read_thread_turns(thread_id)?;
        turns.sort_by(|a, b| {
            b.started_at
                .cmp(&a.started_at)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
        Ok(turns)
    }

    /// Remove every snapshot file, readable or not (per-turn files, thread
    /// directories, and any legacy flat files). Used by `threads_purge` to
    /// guarantee a destructive cleanup leaves nothing — `list()` only returns
    /// parseable snapshots, so iterating list+delete would silently leave
    /// half-written or schema-skewed files behind. Returns the count of JSON
    /// files removed.
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
            let file_type = entry
                .file_type()
                .map_err(|e| format!("stat turn-state entry {}: {e}", path.display()))?;
            if file_type.is_dir() {
                // A per-thread directory: count and remove its JSON files, then
                // drop the (now empty) directory.
                for sub in fs::read_dir(&path)
                    .map_err(|e| format!("read thread dir {}: {e}", path.display()))?
                {
                    let sub = sub.map_err(|e| format!("read thread entry: {e}"))?;
                    let sub_path = sub.path();
                    if sub_path.extension().and_then(|s| s.to_str()) == Some(SNAPSHOT_EXTENSION) {
                        fs::remove_file(&sub_path).map_err(|e| {
                            format!("remove turn-state file {}: {e}", sub_path.display())
                        })?;
                        removed += 1;
                    }
                }
                fs::remove_dir_all(&path)
                    .map_err(|e| format!("remove thread dir {}: {e}", path.display()))?;
            } else if path.extension().and_then(|s| s.to_str()) == Some(SNAPSHOT_EXTENSION) {
                // A legacy flat snapshot.
                fs::remove_file(&path)
                    .map_err(|e| format!("remove turn-state file {}: {e}", path.display()))?;
                removed += 1;
            }
        }
        if removed > 0 {
            debug!(
                "{LOG_PREFIX} cleared {removed} snapshots from {}",
                dir.display()
            );
        }
        Ok(removed)
    }

    /// Mark every non-terminal turn as `Interrupted`. Intended to run on startup
    /// so the UI can distinguish stale turns left behind by a previous process
    /// from turns currently being driven. `Completed`/`Interrupted` turns are
    /// left as-is (idempotent; completed turns are intentionally kept so the
    /// processing panel can replay a finished turn after a reboot).
    pub fn mark_all_interrupted(&self, now_rfc3339: &str) -> Result<usize, String> {
        let turns = {
            let _guard = TURN_STATE_LOCK.lock();
            self.migrate_all_legacy_locked();
            self.all_turns_locked()?
        };
        let mut count = 0usize;
        for mut snapshot in turns {
            if matches!(
                snapshot.lifecycle,
                TurnLifecycle::Interrupted | TurnLifecycle::Completed
            ) {
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

    // --- internals -------------------------------------------------------

    fn ensure_thread_dir(&self, thread_id: &str) -> Result<PathBuf, String> {
        let dir = self.thread_dir(thread_id);
        fs::create_dir_all(&dir)
            .map_err(|e| format!("create thread turn-state dir {}: {e}", dir.display()))?;
        Ok(dir)
    }

    fn dir(&self) -> PathBuf {
        self.workspace_dir
            .join("memory")
            .join("conversations")
            .join(TURN_STATE_DIR)
    }

    fn thread_dir(&self, thread_id: &str) -> PathBuf {
        self.dir().join(hex::encode(thread_id.as_bytes()))
    }

    fn turn_path(&self, thread_id: &str, request_id: &str) -> PathBuf {
        self.thread_dir(thread_id).join(format!(
            "{}.{}",
            hex::encode(request_id.as_bytes()),
            SNAPSHOT_EXTENSION
        ))
    }

    fn legacy_flat_path(&self, thread_id: &str) -> PathBuf {
        self.dir().join(format!(
            "{}.{}",
            hex::encode(thread_id.as_bytes()),
            SNAPSHOT_EXTENSION
        ))
    }

    /// Read every parseable turn snapshot in one thread's directory.
    /// Unreadable files are logged and skipped (mirrors `list()`'s resilience).
    fn read_thread_turns(&self, thread_id: &str) -> Result<Vec<TurnState>, String> {
        let dir = self.thread_dir(thread_id);
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut turns = Vec::new();
        for entry in fs::read_dir(&dir)
            .map_err(|e| format!("read thread turn-state dir {}: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| format!("read thread turn-state entry: {e}"))?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some(SNAPSHOT_EXTENSION) {
                continue;
            }
            match read_snapshot(&path) {
                Ok(snapshot) => turns.push(snapshot),
                Err(err) => warn!(
                    "{LOG_PREFIX} skip unreadable snapshot {}: {err}",
                    path.display()
                ),
            }
        }
        Ok(turns)
    }

    /// hex(thread_id) directory names under the root, decoded back to the
    /// thread-id string. Skips legacy flat files (handled by migration).
    fn thread_ids(&self) -> Result<Vec<String>, String> {
        let dir = self.dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut ids = Vec::new();
        for entry in
            fs::read_dir(&dir).map_err(|e| format!("read turn-state dir {}: {e}", dir.display()))?
        {
            let entry = entry.map_err(|e| format!("read turn-state entry: {e}"))?;
            if !entry
                .file_type()
                .map_err(|e| format!("stat turn-state entry: {e}"))?
                .is_dir()
            {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            match hex::decode(name)
                .ok()
                .and_then(|b| String::from_utf8(b).ok())
            {
                Some(thread_id) => ids.push(thread_id),
                None => warn!("{LOG_PREFIX} skip non-hex thread dir {name}"),
            }
        }
        Ok(ids)
    }

    /// Every turn across every thread. Caller holds the lock.
    fn all_turns_locked(&self) -> Result<Vec<TurnState>, String> {
        let mut turns = Vec::new();
        for thread_id in self.thread_ids()? {
            turns.append(&mut self.read_thread_turns(&thread_id)?);
        }
        Ok(turns)
    }

    /// If a legacy flat file exists for `thread_id`, fold it into the per-turn
    /// layout and remove the flat file. Best-effort; failures are logged. Caller
    /// holds the lock.
    fn migrate_thread_locked(&self, thread_id: &str) {
        let flat = self.legacy_flat_path(thread_id);
        if !flat.exists() {
            return;
        }
        match read_snapshot(&flat) {
            Ok(state) => {
                if let Err(err) = self.write_turn_file(&state) {
                    warn!(
                        "{LOG_PREFIX} legacy migrate write failed thread={thread_id}: {err} (flat file kept)"
                    );
                    return;
                }
                if let Err(err) = fs::remove_file(&flat) {
                    warn!(
                        "{LOG_PREFIX} legacy migrate: removed-into-dir but flat delete failed {}: {err}",
                        flat.display()
                    );
                } else {
                    debug!(
                        "{LOG_PREFIX} migrated legacy snapshot thread={thread_id} request={}",
                        state.request_id
                    );
                }
            }
            Err(err) => warn!(
                "{LOG_PREFIX} legacy migrate: unreadable flat file {} left in place: {err}",
                flat.display()
            ),
        }
    }

    /// Migrate every legacy flat file under the root. Caller holds the lock.
    fn migrate_all_legacy_locked(&self) {
        let dir = self.dir();
        if !dir.exists() {
            return;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                warn!("{LOG_PREFIX} migrate scan failed {}: {err}", dir.display());
                return;
            }
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some(SNAPSHOT_EXTENSION) {
                continue; // directories and non-json files
            }
            match read_snapshot(&path) {
                Ok(state) => {
                    if self.write_turn_file(&state).is_ok() {
                        let _ = fs::remove_file(&path);
                        debug!(
                            "{LOG_PREFIX} migrated legacy snapshot thread={} request={}",
                            state.thread_id, state.request_id
                        );
                    }
                }
                Err(err) => warn!(
                    "{LOG_PREFIX} migrate: unreadable flat file {} left in place: {err}",
                    path.display()
                ),
            }
        }
    }

    /// Atomic per-turn write without migration/retention side effects. Used by
    /// migration to relocate a snapshot. Caller holds the lock.
    fn write_turn_file(&self, state: &TurnState) -> Result<(), String> {
        let dir = self.ensure_thread_dir(&state.thread_id)?;
        let path = self.turn_path(&state.thread_id, &state.request_id);
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
        if let Err(err) = sync_dir(&dir) {
            log::warn!("{LOG_PREFIX} failed to fsync {}: {err}", dir.display());
        }
        Ok(())
    }

    /// Prune a thread's `Completed` turns to the newest [`COMPLETED_RETENTION`]
    /// by `updated_at`. Non-completed turns (at most the one live turn) are kept.
    /// Caller holds the lock. Best-effort — failures are logged, not fatal.
    fn prune_completed_locked(&self, thread_id: &str) {
        let turns = match self.read_thread_turns(thread_id) {
            Ok(turns) => turns,
            Err(err) => {
                warn!("{LOG_PREFIX} prune read failed thread={thread_id}: {err}");
                return;
            }
        };
        let mut completed: Vec<TurnState> = turns
            .into_iter()
            .filter(|t| t.lifecycle == TurnLifecycle::Completed)
            .collect();
        if completed.len() <= COMPLETED_RETENTION {
            return;
        }
        // Newest first, then drop everything past the retention window.
        completed.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        for stale in completed.into_iter().skip(COMPLETED_RETENTION) {
            let path = self.turn_path(&stale.thread_id, &stale.request_id);
            if let Err(err) = fs::remove_file(&path) {
                warn!("{LOG_PREFIX} prune remove failed {}: {err}", path.display());
            } else {
                debug!(
                    "{LOG_PREFIX} pruned completed turn thread={thread_id} request={}",
                    stale.request_id
                );
            }
        }
    }
}

/// Pick the latest turn (greatest `started_at`, ties broken by `updated_at`).
fn latest_turn(turns: Vec<TurnState>) -> Option<TurnState> {
    turns.into_iter().max_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.updated_at.cmp(&b.updated_at))
    })
}

/// Best-effort `fsync` of a directory entry. On Unix, opens the directory for
/// read and calls `sync_all` on the file handle. On Windows this is a no-op —
/// directory fsync is not exposed by the platform and the rename's durability is
/// provided by NTFS journaling.
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

pub fn get_turn(
    workspace_dir: PathBuf,
    thread_id: &str,
    request_id: &str,
) -> Result<Option<TurnState>, String> {
    TurnStateStore::new(workspace_dir).get_turn(thread_id, request_id)
}

pub fn delete(workspace_dir: PathBuf, thread_id: &str) -> Result<bool, String> {
    TurnStateStore::new(workspace_dir).delete(thread_id)
}

pub fn list(workspace_dir: PathBuf) -> Result<Vec<TurnState>, String> {
    TurnStateStore::new(workspace_dir).list()
}

pub fn list_thread(workspace_dir: PathBuf, thread_id: &str) -> Result<Vec<TurnState>, String> {
    TurnStateStore::new(workspace_dir).list_thread(thread_id)
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
