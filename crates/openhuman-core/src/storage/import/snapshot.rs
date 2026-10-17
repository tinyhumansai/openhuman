//! A private, throw-away copy of one legacy database.
//!
//! Reading a SQLite file opens it for writing as far as the legacy readers
//! are concerned (they create missing tables and migrate an old schema), and
//! the storage driver declares its own tables on open. Neither may touch a
//! user's real file, so the importer reads a consistent snapshot in a scratch
//! directory laid out like the workspace.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use tempfile::TempDir;
use tinystoragedrivers::CollectionSpec;

use super::Record;
use crate::storage::rusqlite;

/// A consistent snapshot of one legacy database (WAL contents included)
/// inside a scratch workspace. Removed on
/// drop.
pub struct Snapshot {
    dir: TempDir,
    db: PathBuf,
}

impl Snapshot {
    /// Copies `source` (whose workspace-relative path is `relative`) into a
    /// scratch workspace.
    ///
    /// # Errors
    ///
    /// When the scratch directory or a copy cannot be written, or `source`
    /// cannot be read.
    pub fn of(source: &Path, relative: &str) -> Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix("openhuman-storage-import-")
            .tempdir()
            .context("create the scratch directory")?;
        let db = dir.path().join(relative);
        if let Some(parent) = db.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("create {}", parent.display()))?;
        }
        // `VACUUM INTO` is a SQLite-consistent read: the main file and the
        // WAL are seen as one committed state, which two separate file
        // copies could not guarantee while a writer checkpoints.
        let src = rusqlite::Connection::open_with_flags(
            source,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .with_context(|| format!("open {} read-only", source.display()))?;
        src.execute("VACUUM INTO ?1", [db.to_string_lossy().as_ref()])
            .with_context(|| format!("snapshot {}", source.display()))?;
        Ok(Self { dir, db })
    }

    /// The scratch workspace the copy lives in.
    pub fn workspace(&self) -> &Path {
        self.dir.path()
    }

    /// The copied database file.
    pub fn db(&self) -> &Path {
        &self.db
    }

    /// Every record in the copy's driver document tables, in the collections
    /// `collections` names. A file with none yields nothing.
    ///
    /// # Errors
    ///
    /// When the copy cannot be opened with the SQLite driver or read.
    pub fn driver_docs(&self, collections: fn() -> Vec<CollectionSpec>) -> Result<Vec<Record>> {
        use tinystoragedrivers::{DocumentStoreExt, Query, StorageBackend};

        let storage = tinystoragedrivers::sqlite::SqliteStorage::open(&self.db)
            .with_context(|| format!("open the copy {}", self.db.display()))?;
        let scoped = storage
            .for_scope(&crate::storage::tenant_scope(None))
            .context("open the local scope of the copy")?;
        let docs = std::sync::Arc::clone(scoped.documents());
        let specs = collections();
        let found = crate::storage::block_on(async move {
            let mut out = Vec::new();
            for spec in specs {
                docs.ensure_collection(&spec).await?;
                for stored in docs.query_all(&spec.name, &Query::all()).await? {
                    out.push(Record {
                        collection: spec.name.clone(),
                        id: stored.id,
                        doc: stored.doc,
                    });
                }
            }
            Ok(out)
        })?;
        Ok(found)
    }
}

fn companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(test)]
#[path = "snapshot_tests.rs"]
mod tests;
