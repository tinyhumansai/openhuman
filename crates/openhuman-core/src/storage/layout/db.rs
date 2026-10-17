//! Step 4: the v1 database becomes the profile's `storage.db`.
//!
//! The v1 default is one SQLite file, `workspace/storage/storage.db`, with
//! every scope's rows in it. `VACUUM INTO` writes a consistent, compacted copy
//! (it reads through the WAL), to `storage.db.tmp`; the copy is verified table
//! by table against the source (the driver keeps documents, streams and blobs
//! in tables, so equal row counts mean equal collection, stream and blob
//! counts) and only then renamed into place. The source is never modified.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::state::LayoutError;
use crate::storage::rusqlite::{self, Connection, OpenFlags};

/// Row counts per table.
pub type Counts = BTreeMap<String, u64>;

/// What a copy did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbCopy {
    /// Row counts of the copy (equal to the source's).
    pub counts: Counts,
    /// Whether an existing, matching `storage.db` was kept instead of copying.
    pub reused: bool,
}

fn sql(error: rusqlite::Error) -> LayoutError {
    LayoutError::Other(anyhow::anyhow!("sqlite: {error}"))
}

/// Row counts of every table of the database at `path`.
///
/// # Errors
///
/// When the file cannot be opened or read.
pub fn table_counts(path: &Path) -> Result<Counts, LayoutError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(sql)?;
    counts_of(&conn)
}

fn counts_of(conn: &Connection) -> Result<Counts, LayoutError> {
    let mut tables = Vec::new();
    {
        let mut stmt = conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .map_err(sql)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(sql)?;
        for row in rows {
            tables.push(row.map_err(sql)?);
        }
    }
    let mut counts = Counts::new();
    for table in tables {
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let n: i64 = conn
            .query_row(&format!("SELECT COUNT(*) FROM {quoted}"), [], |row| {
                row.get(0)
            })
            .map_err(sql)?;
        counts.insert(table, u64::try_from(n).unwrap_or(0));
    }
    Ok(counts)
}

fn tmp_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".tmp");
    dest.with_file_name(name)
}

fn mismatch(source: &Counts, copy: &Counts) -> Option<String> {
    if source == copy {
        return None;
    }
    let mut tables: Vec<&String> = source.keys().chain(copy.keys()).collect();
    tables.sort();
    tables.dedup();
    let diffs: Vec<String> = tables
        .into_iter()
        .filter(|t| source.get(*t) != copy.get(*t))
        .map(|t| format!("{t}: source {:?} copy {:?}", source.get(t), copy.get(t)))
        .collect();
    Some(diffs.join("; "))
}

/// Copies `source` to `dest` (see the module docs). Idempotent: a `dest` whose
/// counts already equal the source's is kept; one that differs is replaced (it
/// can only be left over from an interrupted run, because nothing opens it
/// before the layout marker exists).
///
/// # Errors
///
/// [`LayoutError::Verify`] when the copy's counts differ from the source's, or
/// an I/O or SQLite error. The source is untouched either way.
pub fn copy(source: &Path, dest: &Path) -> Result<DbCopy, LayoutError> {
    let conn =
        Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(sql)?;
    let source_counts = counts_of(&conn)?;
    if dest.is_file() {
        if let Ok(existing) = table_counts(dest) {
            if existing == source_counts {
                tracing::debug!(target: "openhuman::storage", dest = %dest.display(), "[layout] storage.db already matches the source; kept");
                return Ok(DbCopy {
                    counts: existing,
                    reused: true,
                });
            }
        }
    }
    let tmp = tmp_path(dest);
    for stale in [
        tmp.clone(),
        tmp.with_extension("tmp-wal"),
        tmp.with_extension("tmp-shm"),
    ] {
        let _ = std::fs::remove_file(stale);
    }
    conn.execute("VACUUM INTO ?1", [tmp.to_string_lossy().as_ref()])
        .map_err(sql)?;
    drop(conn);
    let copy_counts = table_counts(&tmp)?;
    if let Some(diff) = mismatch(&source_counts, &copy_counts) {
        let _ = std::fs::remove_file(&tmp);
        return Err(LayoutError::Verify(format!(
            "the database copy does not match its source ({diff})"
        )));
    }
    std::fs::File::open(&tmp)?.sync_all()?;
    std::fs::rename(&tmp, dest)?;
    tracing::info!(
        target: "openhuman::storage",
        tables = copy_counts.len(),
        rows = copy_counts.values().sum::<u64>(),
        "[layout] database copied and verified"
    );
    Ok(DbCopy {
        counts: copy_counts,
        reused: false,
    })
}

/// Checkpoints and truncates the WAL of `path`, so the file set the archive
/// reads is complete in the `.db` file.
///
/// # Errors
///
/// A SQLite error.
pub fn checkpoint(path: &Path) -> Result<(), LayoutError> {
    let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE).map_err(sql)?;
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
        .map_err(sql)?;
    Ok(())
}

#[cfg(test)]
#[path = "db_tests.rs"]
mod tests;
