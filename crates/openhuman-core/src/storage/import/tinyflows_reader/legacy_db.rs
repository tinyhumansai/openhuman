//! Read retired TinyFlows SQLite files through a consistent scratch snapshot.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::storage::import::Snapshot;
use crate::storage::rusqlite;

/// A consistent scratch snapshot of a legacy database.
pub(super) struct LegacyDb {
    snapshot: Snapshot,
}

impl LegacyDb {
    /// Takes a read-only snapshot of an existing database.
    pub(super) fn open(path: &Path) -> Result<Option<Self>> {
        if !path.is_file() {
            return Ok(None);
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("legacy database path has no UTF-8 file name")?;
        Ok(Some(Self {
            snapshot: Snapshot::of(path, name)?,
        }))
    }

    /// Runs a read query against the private snapshot.
    pub(super) async fn read<T, F>(&self, what: &'static str, read: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let path: PathBuf = self.snapshot.db().to_path_buf();
        // This worker reads only the explicit private snapshot. Tenant-scoped
        // writes happen later through the caller's captured module proxy.
        tokio::task::spawn_blocking(move || {
            let connection = rusqlite::Connection::open_with_flags(
                path,
                rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
            )?;
            read(&connection)
        })
        .await
        .with_context(|| format!("reading {what} from the legacy database"))?
        .map_err(anyhow::Error::from)
    }

    /// Lists tables present in the snapshot.
    pub(super) async fn tables(&self) -> Result<HashSet<String>> {
        self.read("the table list", |connection| {
            let mut statement =
                connection.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<HashSet<_>>>();
            rows
        })
        .await
    }

    /// Lists columns present in a table.
    pub(super) async fn columns(&self, table: &'static str) -> Result<HashSet<String>> {
        self.read("a column list", move |connection| {
            let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<rusqlite::Result<HashSet<_>>>();
            rows
        })
        .await
    }
}

/// Selects the actual column or a literal historical default.
pub(super) fn column_or(have: &HashSet<String>, column: &str, default: &str) -> String {
    if have.contains(column) {
        column.to_string()
    } else {
        format!("{default} AS {column}")
    }
}
