//! SQLite-backed mtime state store for the vault file watcher.
//!
//! Persists `path → last_mtime_secs` across restarts so the watcher can
//! detect real changes on startup without re-ingesting every file from
//! scratch.  One row per tracked file; the table is created lazily on
//! first use.
//!
//! ## Schema
//!
//! ```sql
//! CREATE TABLE IF NOT EXISTS vault_watcher_state (
//!     path       TEXT    PRIMARY KEY,
//!     mtime_secs INTEGER NOT NULL,
//!     deleted    INTEGER NOT NULL DEFAULT 0
//! );
//! ```

use rusqlite::{params, Connection, OptionalExtension, Result as SqlResult};
use std::path::{Path, PathBuf};

pub struct WatcherStateStore {
    conn: Connection,
}

#[derive(Debug, Clone)]
pub struct FileState {
    pub path: PathBuf,
    pub mtime_secs: u64,
    pub deleted: bool,
}

impl WatcherStateStore {
    /// Open (or create) the state database at `db_path`.
    pub fn open(db_path: &Path) -> SqlResult<Self> {
        log::debug!("[watcher::state] open db_path={}", db_path.display());
        let conn = Connection::open(db_path)?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous  = NORMAL;
             CREATE TABLE IF NOT EXISTS vault_watcher_state (
                 path       TEXT    PRIMARY KEY,
                 mtime_secs INTEGER NOT NULL,
                 deleted    INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        log::debug!("[watcher::state] open ok");
        Ok(Self { conn })
    }

    /// Upsert the mtime for `path`, marking it as not-deleted.
    pub fn record_seen(&mut self, path: &Path, mtime_secs: u64) -> SqlResult<()> {
        log::trace!(
            "[watcher::state] record_seen file={} mtime={}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            mtime_secs
        );
        self.conn.execute(
            "INSERT INTO vault_watcher_state (path, mtime_secs, deleted)
             VALUES (?1, ?2, 0)
             ON CONFLICT(path) DO UPDATE SET mtime_secs = ?2, deleted = 0",
            params![path.to_string_lossy().as_ref(), mtime_secs as i64],
        )?;
        Ok(())
    }

    /// Mark `path` as deleted (keeps the row so we don't re-process on restart).
    pub fn record_deleted(&mut self, path: &Path) -> SqlResult<()> {
        log::trace!(
            "[watcher::state] record_deleted file={}",
            path.file_name().unwrap_or_default().to_string_lossy()
        );
        self.conn.execute(
            "INSERT INTO vault_watcher_state (path, mtime_secs, deleted)
             VALUES (?1, 0, 1)
             ON CONFLICT(path) DO UPDATE SET deleted = 1",
            params![path.to_string_lossy().as_ref()],
        )?;
        Ok(())
    }

    /// Return the stored mtime for `path`, or `None` if never seen / deleted.
    pub fn last_mtime(&self, path: &Path) -> SqlResult<Option<u64>> {
        let mut stmt = self.conn.prepare_cached(
            "SELECT mtime_secs, deleted FROM vault_watcher_state WHERE path = ?1",
        )?;
        let result = stmt
            .query_row(params![path.to_string_lossy().as_ref()], |row| {
                let mtime: i64 = row.get(0)?;
                let deleted: bool = row.get(1)?;
                Ok((mtime as u64, deleted))
            })
            .optional()?;

        Ok(result.and_then(|(mtime, deleted)| (!deleted).then_some(mtime)))
    }

    /// Load all non-deleted states — used at startup to seed the in-memory map.
    pub fn load_all(&self) -> SqlResult<Vec<FileState>> {
        let mut stmt = self.conn.prepare(
            "SELECT path, mtime_secs, deleted FROM vault_watcher_state WHERE deleted = 0",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let path: String = row.get(0)?;
                let mtime: i64 = row.get(1)?;
                let deleted: bool = row.get(2)?;
                Ok(FileState {
                    path: PathBuf::from(path),
                    mtime_secs: mtime as u64,
                    deleted,
                })
            })?
            .collect::<SqlResult<Vec<_>>>()?;
        log::debug!("[watcher::state] load_all rows={}", rows.len());
        Ok(rows)
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
