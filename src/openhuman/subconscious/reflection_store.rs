//! SQLite persistence for the proactive subconscious reflection layer (#623).
//!
//! Two tables:
//! - `subconscious_reflections` — durable record of every reflection the
//!   tick LLM emits. Indexed by `(created_at DESC)` so the Intelligence tab
//!   and the prompt's "Recent reflections" section can both fetch in one go.
//! - `subconscious_hotness_snapshots` — per-entity copy of the previous
//!   tick's hotness score, used by the situation report's
//!   `hotness_deltas` section to compute meaningful movement.
//!
//! DDL is appended to `super::store::SCHEMA_DDL` so the schema migration
//! and `with_connection` lifecycle stay unified — no parallel DB handle.
//! See [`super::store::with_connection`] for the sole entry point.
//!
//! Migration note: prior versions of this schema carried `disposition` and
//! `surfaced_at` columns to support the now-removed auto-post-into-thread
//! flow. [`migrate_drop_legacy_columns`] handles existing DBs by dropping
//! those columns + their index; the DDL below describes the post-migration
//! shape so fresh installs come up clean.

use anyhow::{Context, Result};
use rusqlite::{params, Connection, OptionalExtension};

use super::reflection::{Reflection, ReflectionKind};
use super::source_chunk::SourceChunk;

/// DDL appended to the subconscious schema. Imported by `super::store`'s
/// `SCHEMA_DDL` constant so every connection runs the migration.
pub const REFLECTION_SCHEMA_DDL: &str = "
    CREATE TABLE IF NOT EXISTS subconscious_reflections (
        id              TEXT PRIMARY KEY,
        kind            TEXT NOT NULL,
        body            TEXT NOT NULL,
        proposed_action TEXT,
        source_refs     TEXT NOT NULL DEFAULT '[]',
        source_chunks   TEXT NOT NULL DEFAULT '[]',
        created_at      REAL NOT NULL,
        acted_on_at     REAL,
        dismissed_at    REAL
    );
    CREATE INDEX IF NOT EXISTS idx_reflections_created
        ON subconscious_reflections(created_at DESC);

    CREATE TABLE IF NOT EXISTS subconscious_hotness_snapshots (
        entity_id       TEXT PRIMARY KEY,
        score           REAL NOT NULL,
        captured_at     REAL NOT NULL
    );
";

/// Best-effort migration: drop the legacy `disposition` / `surfaced_at`
/// columns and their index from `subconscious_reflections` if they still
/// exist on disk. Idempotent — repeated calls and clean installs are no-ops.
///
/// Each statement is run with errors swallowed because:
/// - On a fresh install the columns/index were never created → DROP errors.
/// - On a previously-migrated install the columns/index are already gone.
/// - SQLite ≥ 3.35 supports `ALTER TABLE ... DROP COLUMN`; older builds
///   would fail this whole block, but we ship sqlite≥3.35 via rusqlite's
///   bundled feature so this is fine in practice.
pub fn migrate_drop_legacy_columns(conn: &Connection) {
    let _ = conn.execute(
        "DROP INDEX IF EXISTS idx_reflections_disposition_created",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE subconscious_reflections DROP COLUMN disposition",
        [],
    );
    let _ = conn.execute(
        "ALTER TABLE subconscious_reflections DROP COLUMN surfaced_at",
        [],
    );
}

/// Idempotent additive migration: add the `source_chunks` JSON column to
/// previously-migrated DBs that pre-date the #623-followup memory-context
/// snapshot work. Errors swallowed because:
/// - Fresh installs already have the column from the CREATE TABLE above.
/// - Already-migrated installs have it too, so ADD COLUMN errors with
///   "duplicate column" — a no-op for our purposes.
pub fn migrate_add_source_chunks_column(conn: &Connection) {
    let _ = conn.execute(
        "ALTER TABLE subconscious_reflections ADD COLUMN source_chunks TEXT NOT NULL DEFAULT '[]'",
        [],
    );
}

// ── Reflection CRUD ──────────────────────────────────────────────────────────

/// Persist a fresh reflection. Idempotent on `id`: if a row with the same
/// id already exists the existing row is preserved (caller should be
/// generating UUIDs, so this is purely a safety net for double-writes).
pub fn add_reflection(conn: &Connection, reflection: &Reflection) -> Result<()> {
    let source_refs_json = serde_json::to_string(&reflection.source_refs)
        .context("serialize source_refs")
        .unwrap_or_else(|_| "[]".to_string());
    let source_chunks_json = serde_json::to_string(&reflection.source_chunks)
        .context("serialize source_chunks")
        .unwrap_or_else(|_| "[]".to_string());
    conn.execute(
        "INSERT OR IGNORE INTO subconscious_reflections (
            id, kind, body, proposed_action, source_refs, source_chunks,
            created_at, acted_on_at, dismissed_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            reflection.id,
            reflection.kind.as_str(),
            reflection.body,
            reflection.proposed_action,
            source_refs_json,
            source_chunks_json,
            reflection.created_at,
            reflection.acted_on_at,
            reflection.dismissed_at,
        ],
    )
    .context("insert reflection")?;
    log::debug!(
        "[subconscious::reflection_store] added id={} kind={} chunks={}",
        reflection.id,
        reflection.kind.as_str(),
        reflection.source_chunks.len()
    );
    Ok(())
}

/// List reflections in reverse-chronological order, with optional cursor
/// `since_ts` (epoch seconds; rows with `created_at > since_ts`).
pub fn list_recent(
    conn: &Connection,
    limit: usize,
    since_ts: Option<f64>,
) -> Result<Vec<Reflection>> {
    let limit = limit.max(1) as i64;
    let mut rows = Vec::new();
    let mut stmt;
    let mapped: Vec<Reflection> = if let Some(ts) = since_ts {
        stmt = conn.prepare(
            "SELECT id, kind, body, proposed_action, source_refs, source_chunks,
                    created_at, acted_on_at, dismissed_at
             FROM subconscious_reflections
             WHERE created_at > ?1
             ORDER BY created_at DESC LIMIT ?2",
        )?;
        let it = stmt.query_map(params![ts, limit], row_to_reflection)?;
        for r in it {
            rows.push(r?);
        }
        rows
    } else {
        stmt = conn.prepare(
            "SELECT id, kind, body, proposed_action, source_refs, source_chunks,
                    created_at, acted_on_at, dismissed_at
             FROM subconscious_reflections
             ORDER BY created_at DESC LIMIT ?1",
        )?;
        let it = stmt.query_map(params![limit], row_to_reflection)?;
        for r in it {
            rows.push(r?);
        }
        rows
    };
    Ok(mapped)
}

/// Fetch one reflection by id.
pub fn get_reflection(conn: &Connection, id: &str) -> Result<Option<Reflection>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, body, proposed_action, source_refs, source_chunks,
                created_at, acted_on_at, dismissed_at
         FROM subconscious_reflections WHERE id = ?1",
    )?;
    let r = stmt
        .query_row(params![id], row_to_reflection)
        .optional()
        .context("get reflection")?;
    Ok(r)
}

/// Stamp `acted_on_at` when the user taps the proposed action.
pub fn mark_acted(conn: &Connection, id: &str, ts: f64) -> Result<()> {
    conn.execute(
        "UPDATE subconscious_reflections SET acted_on_at = ?1 WHERE id = ?2",
        params![ts, id],
    )?;
    Ok(())
}

/// Stamp `dismissed_at` when the user dismisses the card.
pub fn mark_dismissed(conn: &Connection, id: &str, ts: f64) -> Result<()> {
    conn.execute(
        "UPDATE subconscious_reflections SET dismissed_at = ?1 WHERE id = ?2",
        params![ts, id],
    )?;
    Ok(())
}

fn row_to_reflection(row: &rusqlite::Row) -> rusqlite::Result<Reflection> {
    let id: String = row.get(0)?;
    let kind_s: String = row.get(1)?;
    let body: String = row.get(2)?;
    let proposed_action: Option<String> = row.get(3)?;
    let source_refs_json: String = row.get(4)?;
    let source_chunks_json: String = row.get(5)?;
    let created_at: f64 = row.get(6)?;
    let acted_on_at: Option<f64> = row.get(7)?;
    let dismissed_at: Option<f64> = row.get(8)?;

    let source_refs: Vec<String> =
        serde_json::from_str(&source_refs_json).unwrap_or_else(|_| Vec::new());
    let source_chunks: Vec<SourceChunk> =
        serde_json::from_str(&source_chunks_json).unwrap_or_else(|_| Vec::new());

    Ok(Reflection {
        id,
        kind: ReflectionKind::from_str_lossy(&kind_s),
        body,
        proposed_action,
        source_refs,
        source_chunks,
        created_at,
        acted_on_at,
        dismissed_at,
    })
}

// ── Hotness snapshot CRUD ────────────────────────────────────────────────────

/// Read all stored snapshots — keyed by `entity_id`. Returns `(entity_id,
/// score)` pairs. Order is unspecified.
pub fn load_hotness_snapshots(conn: &Connection) -> Result<Vec<(String, f64)>> {
    let mut stmt = conn.prepare("SELECT entity_id, score FROM subconscious_hotness_snapshots")?;
    let it = stmt.query_map([], |row| {
        let id: String = row.get(0)?;
        let score: f64 = row.get(1)?;
        Ok((id, score))
    })?;
    let mut out = Vec::new();
    for r in it {
        out.push(r?);
    }
    Ok(out)
}

/// Replace the snapshot table with a fresh capture of current hotness.
/// Atomic — wrapped in a transaction so partial state never leaks.
pub fn replace_hotness_snapshots(
    conn: &mut Connection,
    snapshots: &[(String, f64)],
    captured_at: f64,
) -> Result<()> {
    let tx = conn.transaction()?;
    tx.execute("DELETE FROM subconscious_hotness_snapshots", [])?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO subconscious_hotness_snapshots (entity_id, score, captured_at)
             VALUES (?1, ?2, ?3)",
        )?;
        for (id, score) in snapshots {
            stmt.execute(params![id, score, captured_at])?;
        }
    }
    tx.commit()?;
    log::debug!(
        "[subconscious::reflection_store] replaced hotness snapshots count={}",
        snapshots.len()
    );
    Ok(())
}

#[cfg(test)]
#[path = "reflection_store_tests.rs"]
mod tests;
