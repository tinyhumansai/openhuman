//! SQLite persistence for the `task_sources` domain.
//!
//! Two tables live in `<workspace>/task_sources/sources.db`:
//!   * `task_sources` — the configured sources (provider + filter +
//!     schedule + routing target).
//!   * `ingested_tasks` — per-(source, external task) dedup ledger with
//!     an edit-aware `content_hash`, plus the normalized task payload so
//!     the UI can list recently ingested items.
//!
//! Mirrors the `cron` domain's `with_connection` + migrate-on-open
//! pattern.

use std::time::Duration;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::openhuman::config::Config;
use crate::openhuman::memory_sync::composio::providers::NormalizedTask;

use super::types::{
    FetchReason, FilterSpec, ProviderSlug, SourceTarget, TaskSource, TaskSourcePatch,
};

/// Compute an edit-aware content hash for a task. Two fetches of the
/// same `external_id` whose title/body/status/updated_at/url differ produce
/// different hashes, so an *edited* upstream item re-ingests.
///
/// `url` is part of the canonical form because it is load-bearing downstream
/// (it lands in the card's `source_metadata`/notes and drives external
/// write-back); a provider that edits the URL without advancing `updated_at`
/// would otherwise leave a stale link on the board.
///
/// Uses SHA-256 over a canonical field-delimited representation so the
/// digest is stable across Rust/toolchain versions (the dedup key is
/// persisted on disk; a non-deterministic hasher would force spurious
/// re-ingests after a toolchain bump).
pub fn content_hash(task: &NormalizedTask) -> String {
    let canonical = format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        task.title,
        task.body.as_deref().unwrap_or(""),
        task.status.as_deref().unwrap_or(""),
        task.updated_at.as_deref().unwrap_or(""),
        task.url.as_deref().unwrap_or(""),
    );
    let digest = Sha256::digest(canonical.as_bytes());
    format!("{digest:x}")
}

/// Insert a new task source.
#[allow(clippy::too_many_arguments)]
pub fn add_source(
    config: &Config,
    provider: ProviderSlug,
    connection_id: Option<String>,
    name: Option<String>,
    filter: FilterSpec,
    interval_secs: u64,
    target: SourceTarget,
    max_tasks_per_fetch: u32,
) -> Result<TaskSource> {
    if filter.provider() != provider {
        anyhow::bail!(
            "filter provider '{}' does not match source provider '{}'",
            filter.provider().as_str(),
            provider.as_str()
        );
    }
    // Normalize blank optional fields to NULL so a whitespace-only
    // connection_id can't masquerade as a real selector (mirrors
    // `update_source`).
    let connection_id = connection_id.filter(|s| !s.trim().is_empty());
    let name = name.filter(|s| !s.trim().is_empty());
    let id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let filter_json = serde_json::to_string(&filter).context("serialize task source filter")?;
    let target_json = serde_json::to_string(&target).context("serialize task source target")?;
    let interval_i64 = i64::try_from(interval_secs)
        .context("task source interval_secs exceeds SQLite INTEGER range")?;

    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO task_sources (
                id, provider, connection_id, name, enabled, filter, interval_secs,
                target, max_tasks_per_fetch, created_at
             ) VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                provider.as_str(),
                connection_id,
                name,
                filter_json,
                interval_i64,
                target_json,
                i64::from(max_tasks_per_fetch),
                now.to_rfc3339(),
            ],
        )
        .context("Failed to insert task source")?;
        Ok(())
    })?;

    get_source(config, &id)
}

pub fn get_source(config: &Config, id: &str) -> Result<TaskSource> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(&format!("{SELECT_SOURCE_COLUMNS} WHERE id = ?1"))?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            map_source_row(row).map_err(Into::into)
        } else {
            anyhow::bail!("Task source '{id}' not found")
        }
    })
}

pub fn list_sources(config: &Config) -> Result<Vec<TaskSource>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(&format!(
            "{SELECT_SOURCE_COLUMNS} ORDER BY created_at ASC, id ASC"
        ))?;
        let rows = stmt.query_map([], map_source_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
}

/// Apply a partial patch to a task source.
///
/// **Implementation note:** this function opens three separate SQLite
/// connections (read-modify-write + read-back). At settings-panel scale the
/// overhead is acceptable, but there is a theoretical TOCTOU window between
/// the initial `get_source` and the subsequent `UPDATE`. A future refactor
/// could fold all three operations into a single `with_connection` call using
/// a SQL `UPDATE … RETURNING` pattern.
pub fn update_source(config: &Config, id: &str, patch: TaskSourcePatch) -> Result<TaskSource> {
    let mut source = get_source(config, id)?;

    if let Some(name) = patch.name {
        source.name = Some(name).filter(|s| !s.trim().is_empty());
    }
    if let Some(enabled) = patch.enabled {
        source.enabled = enabled;
    }
    if let Some(filter) = patch.filter {
        if filter.provider() != source.provider {
            anyhow::bail!(
                "patch filter provider '{}' does not match source provider '{}'",
                filter.provider().as_str(),
                source.provider.as_str()
            );
        }
        source.filter = filter;
    }
    if let Some(interval_secs) = patch.interval_secs {
        source.interval_secs = interval_secs;
    }
    if let Some(target) = patch.target {
        source.target = target;
    }
    if let Some(max) = patch.max_tasks_per_fetch {
        source.max_tasks_per_fetch = max;
    }
    if let Some(connection_id) = patch.connection_id {
        source.connection_id = Some(connection_id).filter(|s| !s.trim().is_empty());
    }
    if let Some(assigned_executor) = patch.assigned_executor {
        source.assigned_executor = Some(assigned_executor).filter(|s| !s.trim().is_empty());
    }

    let filter_json = serde_json::to_string(&source.filter).context("serialize filter")?;
    let target_json = serde_json::to_string(&source.target).context("serialize target")?;
    let interval_i64 = i64::try_from(source.interval_secs)
        .context("task source interval_secs exceeds SQLite INTEGER range")?;

    with_connection(config, |conn| {
        conn.execute(
            "UPDATE task_sources
             SET provider = ?1, connection_id = ?2, name = ?3, enabled = ?4, filter = ?5,
                 interval_secs = ?6, target = ?7, max_tasks_per_fetch = ?8,
                 assigned_executor = ?9
             WHERE id = ?10",
            params![
                source.provider.as_str(),
                source.connection_id,
                source.name,
                if source.enabled { 1 } else { 0 },
                filter_json,
                interval_i64,
                target_json,
                i64::from(source.max_tasks_per_fetch),
                source.assigned_executor,
                id,
            ],
        )
        .context("Failed to update task source")?;
        Ok(())
    })?;

    get_source(config, id)
}

pub fn remove_source(config: &Config, id: &str) -> Result<()> {
    let changed = with_connection(config, |conn| {
        conn.execute("DELETE FROM task_sources WHERE id = ?1", params![id])
            .context("Failed to delete task source")
    })?;
    if changed == 0 {
        anyhow::bail!("Task source '{id}' not found");
    }
    Ok(())
}

/// Update a source's `last_fetch_at` / `last_status` after a fetch pass.
pub fn record_fetch(
    config: &Config,
    id: &str,
    finished_at: DateTime<Utc>,
    reason: FetchReason,
    status: &str,
) -> Result<()> {
    let line = format!("{}: {status}", reason.as_str());
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE task_sources SET last_fetch_at = ?1, last_status = ?2 WHERE id = ?3",
            params![finished_at.to_rfc3339(), line, id],
        )
        .context("Failed to record task source fetch")?;
        Ok(())
    })
}

/// True when this `(source, external_id)` was already ingested with the
/// *same* content hash. A differing hash (edited upstream) returns
/// `false` so the item re-ingests.
pub fn is_ingested(
    config: &Config,
    source_id: &str,
    external_id: &str,
    hash: &str,
) -> Result<bool> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT content_hash FROM ingested_tasks WHERE source_id = ?1 AND external_id = ?2",
        )?;
        let mut rows = stmt.query(params![source_id, external_id])?;
        match rows.next()? {
            Some(row) => {
                let existing: String = row.get(0)?;
                Ok(existing == hash)
            }
            None => Ok(false),
        }
    })
}

/// Record a routed task in the dedup ledger (idempotent upsert).
///
/// `card_id` is the board card UUID returned by `route::add_card`; it is
/// persisted so that a later edit of the same upstream task can remove the
/// stale card before creating a fresh one (preventing duplicate board cards).
pub fn mark_ingested(
    config: &Config,
    source_id: &str,
    task: &NormalizedTask,
    card_id: &str,
) -> Result<()> {
    let hash = content_hash(task);
    let payload = serde_json::to_string(task).context("serialize ingested task payload")?;
    let now = Utc::now().to_rfc3339();
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO ingested_tasks (source_id, external_id, content_hash, title, payload, ingested_at, card_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(source_id, external_id) DO UPDATE SET
                content_hash = excluded.content_hash,
                title = excluded.title,
                payload = excluded.payload,
                ingested_at = excluded.ingested_at,
                card_id = excluded.card_id",
            params![source_id, task.external_id, hash, task.title, payload, now, card_id],
        )
        .context("Failed to mark task ingested")?;
        Ok(())
    })
}

/// Return the board card id previously stored for `(source_id, external_id)`,
/// if any. Used by the pipeline to remove stale board cards when an upstream
/// task is edited and re-ingested.
pub fn get_card_id(config: &Config, source_id: &str, external_id: &str) -> Result<Option<String>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT card_id FROM ingested_tasks WHERE source_id = ?1 AND external_id = ?2",
        )?;
        let mut rows = stmt.query(params![source_id, external_id])?;
        match rows.next()? {
            Some(row) => Ok(row.get(0)?),
            None => Ok(None),
        }
    })
}

/// List the most recently ingested tasks for a source (newest first).
pub fn list_ingested(
    config: &Config,
    source_id: &str,
    limit: usize,
) -> Result<Vec<NormalizedTask>> {
    // Floor of 1: a caller passing `limit = 0` still gets at least one row
    // rather than a confusing empty result; `unwrap_or(50)` is the fallback
    // in the unlikely event that `limit` exceeds `i64::MAX`.
    let lim = i64::try_from(limit.max(1)).unwrap_or(50);
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT payload FROM ingested_tasks
             WHERE source_id = ?1 AND payload IS NOT NULL
             ORDER BY ingested_at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![source_id, lim], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            let payload = row?;
            if let Ok(task) = serde_json::from_str::<NormalizedTask>(&payload) {
                out.push(task);
            }
        }
        Ok(out)
    })
}

/// Delete every task source (+ cascade ingested rows). Used by the E2E
/// `test_reset` RPC.
pub fn clear_all(config: &Config) -> Result<usize> {
    with_connection(config, |conn| {
        let removed = conn
            .execute("DELETE FROM task_sources", params![])
            .context("Failed to clear task sources")?;
        // ingested_tasks rows cascade via the FK.
        Ok(removed)
    })
}

const SELECT_SOURCE_COLUMNS: &str = "SELECT id, provider, connection_id, name, enabled, filter, \
     interval_secs, target, max_tasks_per_fetch, created_at, last_fetch_at, last_status, \
     assigned_executor \
     FROM task_sources";

fn map_source_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskSource> {
    let provider_raw: String = row.get(1)?;
    let provider = ProviderSlug::parse(&provider_raw).map_err(sql_conv)?;

    let filter_raw: String = row.get(5)?;
    let filter: FilterSpec = serde_json::from_str(&filter_raw)
        .map_err(|e| sql_conv(format!("invalid filter json: {e}")))?;

    let target_raw: String = row.get(7)?;
    let target: SourceTarget = serde_json::from_str(&target_raw)
        .map_err(|e| sql_conv(format!("invalid target json: {e}")))?;

    let created_at_raw: String = row.get(9)?;
    let last_fetch_raw: Option<String> = row.get(10)?;

    Ok(TaskSource {
        id: row.get(0)?,
        provider,
        connection_id: row.get(2)?,
        name: row.get(3)?,
        enabled: row.get::<_, i64>(4)? != 0,
        filter,
        interval_secs: u64::try_from(row.get::<_, i64>(6)?)
            .map_err(|_| sql_conv("invalid negative interval_secs in task_sources DB"))?,
        target,
        max_tasks_per_fetch: u32::try_from(row.get::<_, i64>(8)?)
            .map_err(|_| sql_conv("invalid max_tasks_per_fetch in task_sources DB"))?,
        assigned_executor: row.get(12)?,
        created_at: parse_rfc3339(&created_at_raw).map_err(sql_conv)?,
        last_fetch_at: match last_fetch_raw {
            Some(raw) => Some(parse_rfc3339(&raw).map_err(sql_conv)?),
            None => None,
        },
        last_status: row.get(11)?,
    })
}

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(raw)
        .with_context(|| format!("Invalid RFC3339 timestamp in task_sources DB: {raw}"))?;
    Ok(parsed.with_timezone(&Utc))
}

fn sql_conv<E: std::fmt::Display>(err: E) -> rusqlite::Error {
    rusqlite::Error::ToSqlConversionFailure(anyhow::anyhow!("{err}").into())
}

fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let db_path = config.workspace_dir.join("task_sources").join("sources.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Failed to create task_sources directory: {}",
                parent.display()
            )
        })?;
    }
    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open task_sources DB: {}", db_path.display()))?;
    // Overlapping periodic-poll + UI writes can otherwise hit
    // "database is locked"; WAL + a busy timeout match the other stores.
    conn.busy_timeout(Duration::from_secs(5))
        .context("Failed to configure task_sources DB busy timeout")?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .context("Failed to enable WAL for task_sources DB")?;

    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         CREATE TABLE IF NOT EXISTS task_sources (
            id                  TEXT PRIMARY KEY,
            provider            TEXT NOT NULL,
            connection_id       TEXT,
            name                TEXT,
            enabled             INTEGER NOT NULL DEFAULT 1,
            filter              TEXT NOT NULL,
            interval_secs       INTEGER NOT NULL,
            target              TEXT NOT NULL,
            max_tasks_per_fetch INTEGER NOT NULL,
            created_at          TEXT NOT NULL,
            last_fetch_at       TEXT,
            last_status         TEXT,
            assigned_executor   TEXT
         );
         CREATE TABLE IF NOT EXISTS ingested_tasks (
            source_id    TEXT NOT NULL,
            external_id  TEXT NOT NULL,
            content_hash TEXT NOT NULL,
            title        TEXT,
            payload      TEXT,
            ingested_at  TEXT NOT NULL,
            card_id      TEXT,
            PRIMARY KEY (source_id, external_id),
            FOREIGN KEY (source_id) REFERENCES task_sources(id) ON DELETE CASCADE
         );
         CREATE INDEX IF NOT EXISTS idx_ingested_source ON ingested_tasks(source_id, ingested_at);",
    )
    .context("Failed to initialize task_sources schema")?;

    // Additive migration: add card_id to existing databases that pre-date
    // this column. Tolerate "duplicate column" in case of a concurrent open.
    add_column_if_missing(&conn, "ingested_tasks", "card_id", "TEXT")?;
    // G7: static executor routing on a source.
    add_column_if_missing(&conn, "task_sources", "assigned_executor", "TEXT")?;

    f(&conn)
}

/// Add a column to a table only when it is absent. Mirrors the pattern used
/// in `cron/store.rs` to keep migrations idempotent across DB versions.
fn add_column_if_missing(
    conn: &Connection,
    table: &str,
    column: &str,
    sql_type: &str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        let col_name: String = row.get(1)?;
        if col_name == column {
            return Ok(()); // already present
        }
    }
    drop(rows);
    drop(stmt);
    match conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {column} {sql_type}"),
        [],
    ) {
        Ok(_) => Ok(()),
        Err(rusqlite::Error::SqliteFailure(_, Some(ref msg)))
            if msg.contains("duplicate column name") =>
        {
            tracing::debug!(
                "[task_sources:store] column {table}.{column} already exists (concurrent migration)"
            );
            Ok(())
        }
        Err(e) => Err(e).with_context(|| format!("Failed to add {table}.{column}")),
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
