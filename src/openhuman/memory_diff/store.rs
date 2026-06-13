//! SQLite persistence for memory diff snapshots and checkpoints.
//!
//! Own database at `<workspace>/memory_diff/diff.db`. Follows the
//! `subconscious/store.rs` self-contained pattern: opens the database,
//! runs DDL on every connection, and provides pure functions.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::time::Duration;

use super::types::{Checkpoint, Snapshot, SnapshotItem, SnapshotTrigger};

const BUSY_TIMEOUT: Duration = Duration::from_millis(5000);
const OPEN_RETRY_ATTEMPTS: u32 = 3;
const OPEN_RETRY_BASE_MS: u64 = 100;

const SCHEMA_DDL: &str = "
CREATE TABLE IF NOT EXISTS mem_diff_snapshots (
    id              TEXT PRIMARY KEY,
    source_id       TEXT NOT NULL,
    source_kind     TEXT NOT NULL,
    label           TEXT NOT NULL,
    trigger_kind    TEXT NOT NULL DEFAULT 'auto',
    item_count      INTEGER NOT NULL,
    taken_at_ms     INTEGER NOT NULL,
    created_at_ms   INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_snap_source
    ON mem_diff_snapshots(source_id, taken_at_ms);

CREATE TABLE IF NOT EXISTS mem_diff_snapshot_items (
    snapshot_id     TEXT NOT NULL REFERENCES mem_diff_snapshots(id) ON DELETE CASCADE,
    item_id         TEXT NOT NULL,
    title           TEXT NOT NULL DEFAULT '',
    content_hash    TEXT NOT NULL,
    timestamp_ms    INTEGER,
    chunk_count     INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (snapshot_id, item_id)
);

CREATE TABLE IF NOT EXISTS mem_diff_checkpoints (
    id              TEXT PRIMARY KEY,
    label           TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS mem_diff_checkpoint_snapshots (
    checkpoint_id   TEXT NOT NULL REFERENCES mem_diff_checkpoints(id) ON DELETE CASCADE,
    snapshot_id     TEXT NOT NULL REFERENCES mem_diff_snapshots(id) ON DELETE CASCADE,
    PRIMARY KEY (checkpoint_id, snapshot_id)
);
";

pub fn with_connection<T>(
    workspace_dir: &Path,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let db_path = workspace_dir.join("memory_diff").join("diff.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create memory_diff dir: {}", parent.display()))?;
    }
    let conn = open_with_retry(&db_path)?;
    f(&conn)
}

fn open_with_retry(db_path: &Path) -> Result<Connection> {
    let mut last_err: Option<anyhow::Error> = None;

    for attempt in 0..=OPEN_RETRY_ATTEMPTS {
        match open_and_init(db_path) {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                if !is_sqlite_busy(&e) || attempt == OPEN_RETRY_ATTEMPTS {
                    last_err = Some(e);
                    break;
                }
                let sleep_ms = OPEN_RETRY_BASE_MS
                    .saturating_mul(3u64.saturating_pow(attempt))
                    .min(900);
                tracing::warn!(
                    attempt = attempt + 1,
                    sleep_ms = sleep_ms,
                    "[memory_diff::store] SQLite busy on open; retrying"
                );
                std::thread::sleep(Duration::from_millis(sleep_ms));
                last_err = Some(e);
            }
        }
    }
    Err(last_err.expect("at least one attempt"))
}

fn open_and_init(db_path: &Path) -> Result<Connection> {
    let conn = Connection::open(db_path)
        .with_context(|| format!("failed to open memory_diff DB: {}", db_path.display()))?;
    conn.busy_timeout(BUSY_TIMEOUT)
        .context("configure memory_diff busy_timeout")?;
    apply_journal_mode(&conn);
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("enable foreign keys")?;
    conn.execute_batch(SCHEMA_DDL)
        .context("failed to run memory_diff schema DDL")?;
    Ok(conn)
}

fn apply_journal_mode(conn: &Connection) {
    match conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0)) {
        Ok(mode) if mode.eq_ignore_ascii_case("wal") => {}
        Ok(_mode) => {
            let _ = conn.pragma_update_and_check(None, "journal_mode", "TRUNCATE", |r| {
                r.get::<_, String>(0)
            });
        }
        Err(_) => {
            let _ = conn.pragma_update_and_check(None, "journal_mode", "TRUNCATE", |r| {
                r.get::<_, String>(0)
            });
        }
    }
}

fn is_sqlite_busy(e: &anyhow::Error) -> bool {
    if let Some(sqlite_err) = e.downcast_ref::<rusqlite::Error>() {
        matches!(
            sqlite_err,
            rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code: rusqlite::ffi::ErrorCode::DatabaseBusy
                        | rusqlite::ffi::ErrorCode::DatabaseLocked,
                    ..
                },
                _
            )
        )
    } else {
        false
    }
}

// ── Snapshot CRUD ──────────────────────────────────────────────────────

pub fn insert_snapshot(
    conn: &Connection,
    snapshot: &Snapshot,
    items: &[SnapshotItem],
) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    let now_ms = chrono::Utc::now().timestamp_millis();
    tx.execute(
        "INSERT INTO mem_diff_snapshots (id, source_id, source_kind, label, trigger_kind, item_count, taken_at_ms, created_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            snapshot.id,
            snapshot.source_id,
            snapshot.source_kind,
            snapshot.label,
            snapshot.trigger.as_str(),
            snapshot.item_count,
            snapshot.taken_at_ms,
            now_ms,
        ],
    )?;
    for item in items {
        tx.execute(
            "INSERT INTO mem_diff_snapshot_items (snapshot_id, item_id, title, content_hash, timestamp_ms, chunk_count)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.id,
                item.item_id,
                item.title,
                item.content_hash,
                item.timestamp_ms,
                item.chunk_count,
            ],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn list_snapshots(
    conn: &Connection,
    source_id: Option<&str>,
    limit: u32,
) -> Result<Vec<Snapshot>> {
    let (sql, params_vec): (String, Vec<Box<dyn rusqlite::ToSql>>) = match source_id {
        Some(sid) => (
            "SELECT id, source_id, source_kind, label, trigger_kind, item_count, taken_at_ms \
             FROM mem_diff_snapshots WHERE source_id = ?1 ORDER BY taken_at_ms DESC LIMIT ?2"
                .to_string(),
            vec![
                Box::new(sid.to_string()) as Box<dyn rusqlite::ToSql>,
                Box::new(limit),
            ],
        ),
        None => (
            "SELECT id, source_id, source_kind, label, trigger_kind, item_count, taken_at_ms \
             FROM mem_diff_snapshots ORDER BY taken_at_ms DESC LIMIT ?1"
                .to_string(),
            vec![Box::new(limit) as Box<dyn rusqlite::ToSql>],
        ),
    };
    let params_refs: Vec<&dyn rusqlite::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(params_refs.as_slice(), |r| {
        Ok(Snapshot {
            id: r.get(0)?,
            source_id: r.get(1)?,
            source_kind: r.get(2)?,
            label: r.get(3)?,
            trigger: parse_trigger(r.get::<_, String>(4)?.as_str()),
            item_count: r.get::<_, i64>(5)? as u32,
            taken_at_ms: r.get(6)?,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn get_snapshot(conn: &Connection, snapshot_id: &str) -> Result<Option<Snapshot>> {
    conn.query_row(
        "SELECT id, source_id, source_kind, label, trigger_kind, item_count, taken_at_ms \
         FROM mem_diff_snapshots WHERE id = ?1",
        [snapshot_id],
        |r| {
            Ok(Snapshot {
                id: r.get(0)?,
                source_id: r.get(1)?,
                source_kind: r.get(2)?,
                label: r.get(3)?,
                trigger: parse_trigger(r.get::<_, String>(4)?.as_str()),
                item_count: r.get::<_, i64>(5)? as u32,
                taken_at_ms: r.get(6)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

pub fn get_snapshot_items(conn: &Connection, snapshot_id: &str) -> Result<Vec<SnapshotItem>> {
    let mut stmt = conn.prepare(
        "SELECT item_id, title, content_hash, timestamp_ms, chunk_count \
         FROM mem_diff_snapshot_items WHERE snapshot_id = ?1 ORDER BY item_id",
    )?;
    let rows = stmt.query_map([snapshot_id], |r| {
        Ok(SnapshotItem {
            item_id: r.get(0)?,
            title: r.get(1)?,
            content_hash: r.get(2)?,
            timestamp_ms: r.get(3)?,
            chunk_count: r.get::<_, i64>(4)? as u32,
        })
    })?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

pub fn latest_snapshots_for_source(
    conn: &Connection,
    source_id: &str,
    count: u32,
) -> Result<Vec<Snapshot>> {
    list_snapshots(conn, Some(source_id), count)
}

// ── Checkpoint CRUD ───────────────────────────────────────────────────

pub fn insert_checkpoint(conn: &Connection, checkpoint: &Checkpoint) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute(
        "INSERT INTO mem_diff_checkpoints (id, label, created_at_ms) VALUES (?1, ?2, ?3)",
        params![checkpoint.id, checkpoint.label, checkpoint.created_at_ms],
    )?;
    for snap_id in &checkpoint.snapshot_ids {
        tx.execute(
            "INSERT INTO mem_diff_checkpoint_snapshots (checkpoint_id, snapshot_id) VALUES (?1, ?2)",
            params![checkpoint.id, snap_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn list_checkpoints(conn: &Connection, limit: u32) -> Result<Vec<Checkpoint>> {
    let mut stmt = conn.prepare(
        "SELECT id, label, created_at_ms FROM mem_diff_checkpoints ORDER BY created_at_ms DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?,
        ))
    })?;
    let mut checkpoints = Vec::new();
    for row in rows {
        let (id, label, created_at_ms) = row?;
        let snap_ids = checkpoint_snapshot_ids(conn, &id)?;
        checkpoints.push(Checkpoint {
            id,
            label,
            created_at_ms,
            snapshot_ids: snap_ids,
        });
    }
    Ok(checkpoints)
}

pub fn get_checkpoint(conn: &Connection, checkpoint_id: &str) -> Result<Option<Checkpoint>> {
    let row: Option<(String, String, i64)> = conn
        .query_row(
            "SELECT id, label, created_at_ms FROM mem_diff_checkpoints WHERE id = ?1",
            [checkpoint_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    match row {
        Some((id, label, created_at_ms)) => {
            let snap_ids = checkpoint_snapshot_ids(conn, &id)?;
            Ok(Some(Checkpoint {
                id,
                label,
                created_at_ms,
                snapshot_ids: snap_ids,
            }))
        }
        None => Ok(None),
    }
}

fn checkpoint_snapshot_ids(conn: &Connection, checkpoint_id: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT snapshot_id FROM mem_diff_checkpoint_snapshots WHERE checkpoint_id = ?1",
    )?;
    let rows = stmt.query_map([checkpoint_id], |r| r.get(0))?;
    rows.collect::<Result<Vec<String>, _>>().map_err(Into::into)
}

// ── Cleanup ───────────────────────────────────────────────────────────

pub fn cleanup_old_snapshots(
    conn: &Connection,
    older_than_ms: i64,
    max_per_source: u32,
) -> Result<u64> {
    let age_deleted: i64 = conn.execute(
        "DELETE FROM mem_diff_snapshots WHERE taken_at_ms < ?1",
        params![older_than_ms],
    )? as i64;

    let trim_deleted: i64 = conn.execute(
        "DELETE FROM mem_diff_snapshots WHERE id IN (
            SELECT id FROM (
                SELECT id, ROW_NUMBER() OVER (
                    PARTITION BY source_id ORDER BY taken_at_ms DESC
                ) as rn FROM mem_diff_snapshots
            ) WHERE rn > ?1
        )",
        params![max_per_source],
    )? as i64;

    Ok((age_deleted + trim_deleted).max(0) as u64)
}

fn parse_trigger(s: &str) -> SnapshotTrigger {
    match s {
        "manual" => SnapshotTrigger::Manual,
        _ => SnapshotTrigger::Auto,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        conn.execute_batch(SCHEMA_DDL).unwrap();
        conn
    }

    fn test_snapshot(id: &str, source_id: &str) -> Snapshot {
        Snapshot {
            id: id.to_string(),
            source_id: source_id.to_string(),
            source_kind: "folder".to_string(),
            label: "Test".to_string(),
            trigger: SnapshotTrigger::Auto,
            item_count: 2,
            taken_at_ms: 1000,
        }
    }

    fn test_items() -> Vec<SnapshotItem> {
        vec![
            SnapshotItem {
                item_id: "file_a".to_string(),
                title: "File A".to_string(),
                content_hash: "aaa".to_string(),
                timestamp_ms: Some(900),
                chunk_count: 1,
            },
            SnapshotItem {
                item_id: "file_b".to_string(),
                title: "File B".to_string(),
                content_hash: "bbb".to_string(),
                timestamp_ms: Some(950),
                chunk_count: 2,
            },
        ]
    }

    #[test]
    fn insert_and_retrieve_snapshot() {
        let conn = test_conn();
        let snap = test_snapshot("snap_1", "src_x");
        let items = test_items();
        insert_snapshot(&conn, &snap, &items).unwrap();

        let loaded = get_snapshot(&conn, "snap_1").unwrap().unwrap();
        assert_eq!(loaded.source_id, "src_x");
        assert_eq!(loaded.item_count, 2);

        let loaded_items = get_snapshot_items(&conn, "snap_1").unwrap();
        assert_eq!(loaded_items.len(), 2);
        assert_eq!(loaded_items[0].item_id, "file_a");
    }

    #[test]
    fn list_snapshots_filters_by_source() {
        let conn = test_conn();
        insert_snapshot(&conn, &test_snapshot("s1", "src_a"), &[]).unwrap();
        insert_snapshot(&conn, &test_snapshot("s2", "src_b"), &[]).unwrap();

        let all = list_snapshots(&conn, None, 100).unwrap();
        assert_eq!(all.len(), 2);

        let filtered = list_snapshots(&conn, Some("src_a"), 100).unwrap();
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].id, "s1");
    }

    #[test]
    fn checkpoint_round_trip() {
        let conn = test_conn();
        insert_snapshot(&conn, &test_snapshot("s1", "src_a"), &[]).unwrap();
        insert_snapshot(&conn, &test_snapshot("s2", "src_b"), &[]).unwrap();

        let ckpt = Checkpoint {
            id: "ckpt_1".to_string(),
            label: "v1".to_string(),
            created_at_ms: 2000,
            snapshot_ids: vec!["s1".to_string(), "s2".to_string()],
        };
        insert_checkpoint(&conn, &ckpt).unwrap();

        let loaded = get_checkpoint(&conn, "ckpt_1").unwrap().unwrap();
        assert_eq!(loaded.label, "v1");
        assert_eq!(loaded.snapshot_ids.len(), 2);

        let all = list_checkpoints(&conn, 10).unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn cleanup_removes_old_snapshots() {
        let conn = test_conn();
        let mut s1 = test_snapshot("s1", "src_a");
        s1.taken_at_ms = 100;
        let mut s2 = test_snapshot("s2", "src_a");
        s2.taken_at_ms = 2000;
        insert_snapshot(&conn, &s1, &test_items()).unwrap();
        insert_snapshot(&conn, &s2, &[]).unwrap();

        let deleted = cleanup_old_snapshots(&conn, 500, 100).unwrap();
        assert_eq!(deleted, 1);

        let remaining = list_snapshots(&conn, None, 100).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "s2");

        // Items were cascade-deleted
        let items = get_snapshot_items(&conn, "s1").unwrap();
        assert!(items.is_empty());
    }
}
