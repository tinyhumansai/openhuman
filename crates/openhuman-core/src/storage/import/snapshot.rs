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
        let conn = rusqlite::Connection::open_with_flags(
            &self.db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .with_context(|| format!("open the copy {} read-only", self.db.display()))?;
        let table = if table_exists(&conn, "docs")? {
            Some(("docs", "body"))
        } else if table_exists(&conn, "_tsd_docs")? {
            Some(("_tsd_docs", "doc"))
        } else {
            None
        };
        let Some((table, body_column)) = table else {
            return Ok(Vec::new());
        };
        let columns = table_columns(&conn, table)?;
        for required in ["scope", "coll", "id", body_column] {
            if !columns.contains(required) {
                anyhow::bail!("legacy driver table {table} is missing {required}");
            }
        }

        // `table` and `body_column` are selected from the fixed names above;
        // collection names remain SQL parameters. Both the v0 document table
        // and layout-2 table are only read from the scratch snapshot.
        let metadata = |name: &str| {
            if columns.contains(name) {
                format!("\"{name}\"")
            } else {
                "NULL".to_owned()
            }
        };
        let deleted = if columns.contains("deleted") {
            " AND COALESCE(deleted, 0) = 0"
        } else {
            ""
        };
        let sql = format!(
            "SELECT coll, id, \"{body_column}\", {}, {}, {}, {} FROM \"{table}\" WHERE scope = ?1 AND coll = ?2{deleted} ORDER BY id",
            metadata("schema"),
            metadata("created_ms"),
            metadata("updated_ms"),
            metadata("expires_ms"),
        );
        let mut out = Vec::new();
        for spec in collections() {
            read_collection(&conn, &sql, &spec.name, &mut out)?;
        }
        Ok(out)
    }
}

fn table_exists(conn: &rusqlite::Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
    .context("inspect legacy driver tables")
}

fn table_columns(
    conn: &rusqlite::Connection,
    table: &str,
) -> Result<std::collections::HashSet<String>> {
    let mut statement = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .context("inspect legacy driver document columns")?;
    let columns = statement
        .query_map([], |row| row.get::<_, String>(1))
        .context("read legacy driver document columns")?
        .collect::<rusqlite::Result<std::collections::HashSet<_>>>()
        .context("collect legacy driver document columns")?;
    Ok(columns)
}

fn read_collection(
    conn: &rusqlite::Connection,
    sql: &str,
    collection: &str,
    out: &mut Vec<Record>,
) -> Result<()> {
    use rusqlite::types::Value as SqlValue;

    let mut statement = conn
        .prepare(sql)
        .context("prepare legacy driver document query")?;
    let rows = statement
        .query_map(["local", collection], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, SqlValue>(2)?,
                row.get::<_, Option<i64>>(3)?,
                row.get::<_, Option<i64>>(4)?,
                row.get::<_, Option<i64>>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })
        .context("query legacy driver documents")?;
    for row in rows {
        let (stored_collection, id, body, schema, created_ms, updated_ms, expires_ms) =
            row.context("read legacy driver document")?;
        out.push(Record {
            collection: stored_collection,
            id,
            doc: decode_legacy_body(body)?,
            created_ms,
            schema: schema.and_then(|value| u32::try_from(value).ok()),
            expires_ms,
            updated_ms,
        });
    }
    Ok(())
}

fn decode_legacy_body(body: rusqlite::types::Value) -> Result<serde_json::Value> {
    match body {
        rusqlite::types::Value::Text(text) => {
            serde_json::from_str(&text).context("decode legacy driver document JSON")
        }
        rusqlite::types::Value::Blob(bytes) if bytes.first() == Some(&1) => {
            let json =
                zstd::decode_all(&bytes[1..]).context("decompress legacy driver document")?;
            serde_json::from_slice(&json).context("decode legacy driver document JSON")
        }
        rusqlite::types::Value::Blob(_) => anyhow::bail!("unknown legacy driver body encoding"),
        _ => anyhow::bail!("legacy driver document body is not text or blob"),
    }
}

fn companion(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}
