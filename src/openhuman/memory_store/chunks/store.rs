//! SQLite-backed persistence for ingested chunks (Phase 1 / issue #707).
//!
//! The store lives at `<workspace>/memory_tree/chunks.db`. Schema is applied
//! lazily on first access via `with_connection`, so the DB is created on
//! demand without an explicit migration step.
//!
//! Upsert semantics: writes are idempotent on `chunk.id` so re-ingesting the
//! same raw source yields no duplicates.
//!
//! ## Connection cache (#2206)
//!
//! `with_connection()` previously opened a new SQLite connection and re-ran
//! the full schema init (8 tables, 15+ indexes, 8+ migrations) on **every**
//! call. With 4 workers polling every 5 s this amounted to ~69K connection
//! opens/day, and a family of WAL/SHM cold-start I/O codes (1546
//! IOERR_TRUNCATE, 4618 IOERR_SHMOPEN, 4874 IOERR_SHMSIZE, 14 CANTOPEN)
//! flooded Sentry with ~19K events in 4 days.
//!
//! Fix: a process-level `ConnectionCache` keyed by DB path. Each entry holds
//! one `parking_lot::Mutex<Connection>` that is initialised once (schema +
//! migrations + legacy-embedding migration) and then reused for all subsequent
//! calls. A per-entry `CircuitBreaker` stops retrying after 3 consecutive
//! init failures for 30 s so a broken install does not busy-loop.

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use parking_lot::Mutex as PMutex;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
#[cfg(test)]
use std::sync::Mutex;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::types::{Chunk, Metadata, SourceKind, SourceRef};
use crate::openhuman::memory_store::content::StagedChunk;

const DB_DIR: &str = "memory_tree";
const DB_FILE: &str = "chunks.db";
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 10_000;
// 15s gives the busy-handler enough headroom that transient write-lock
// contention (4 job workers + scheduler + ingest producers all writing the
// same `memory_tree/chunks.db`) is absorbed inside rusqlite instead of
// surfacing as `SQLITE_BUSY` to callers. Workers still treat busy as a
// soft signal (see `memory::tree::jobs::worker`) so even if this is
// exceeded, the only effect is a one-poll backoff — but 15s is
// comfortably above realistic peer-write durations and shrinks the rate
// at which we have to fall back to that path. The previous 5s was tight
// enough on contended Windows hosts that we were observing avoidable
// busy returns (see OPENHUMAN-TAURI-BP).
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(15);

/// Chunk lifecycle: freshly persisted, awaiting the async extract job.
pub const CHUNK_STATUS_PENDING_EXTRACTION: &str = "pending_extraction";
/// Chunk lifecycle: extract ran and the chunk passed admission.
pub const CHUNK_STATUS_ADMITTED: &str = "admitted";
/// Chunk lifecycle: appended to the L0 buffer of its source tree.
pub const CHUNK_STATUS_BUFFERED: &str = "buffered";
/// Chunk lifecycle: rolled into a sealed L1 summary.
pub const CHUNK_STATUS_SEALED: &str = "sealed";
/// Chunk lifecycle: rejected by the admission gate (too low signal).
pub const CHUNK_STATUS_DROPPED: &str = "dropped";

// `PRAGMA foreign_keys = ON` is intentionally NOT in SCHEMA — it is
// a connection-local pragma that resets to off on every new
// `Connection::open`. SCHEMA only runs once per DB path (first-init);
// applying foreign_keys here would leak FK-off into every later
// `with_connection()` call that hits the fast path. The pragma is
// set per-connection in `open_connection()` instead.

/// `PRAGMA user_version` value once the one-shot legacy→sidecar embedding
/// migration (#1574 §7) has run. `0` (fresh/legacy DB) triggers the copy on
/// next open; `>= 1` skips it. Bump only for a new one-shot data migration.
const TREE_EMBEDDING_MIGRATION_VERSION: i64 = 1;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS mem_tree_chunks (
    id                     TEXT PRIMARY KEY,
    source_kind            TEXT NOT NULL,
    source_id              TEXT NOT NULL,
    source_ref             TEXT,
    owner                  TEXT NOT NULL,
    timestamp_ms           INTEGER NOT NULL,
    time_range_start_ms    INTEGER NOT NULL,
    time_range_end_ms      INTEGER NOT NULL,
    tags_json              TEXT NOT NULL DEFAULT '[]',
    content                TEXT NOT NULL,
    token_count            INTEGER NOT NULL,
    seq_in_source          INTEGER NOT NULL,
    created_at_ms          INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_chunks_source
    ON mem_tree_chunks(source_kind, source_id);
CREATE INDEX IF NOT EXISTS idx_mem_tree_chunks_timestamp
    ON mem_tree_chunks(timestamp_ms);
CREATE INDEX IF NOT EXISTS idx_mem_tree_chunks_owner
    ON mem_tree_chunks(owner);
CREATE INDEX IF NOT EXISTS idx_mem_tree_chunks_source_seq
    ON mem_tree_chunks(source_kind, source_id, seq_in_source);

-- Per-(chunk, embedding model) vectors (#1574). The legacy
-- mem_tree_chunks.embedding column remains in place during the dual-write
-- migration; this table lets multiple vector spaces coexist safely.
CREATE TABLE IF NOT EXISTS mem_tree_chunk_embeddings (
    chunk_id               TEXT NOT NULL REFERENCES mem_tree_chunks(id) ON DELETE CASCADE,
    model_signature        TEXT NOT NULL,
    vector                 BLOB NOT NULL,
    dim                    INTEGER NOT NULL,
    created_at             REAL NOT NULL,
    PRIMARY KEY (chunk_id, model_signature)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_chunk_embeddings_model
    ON mem_tree_chunk_embeddings(model_signature);

-- #1574 §6 reembed-backfill terminal-skip tombstone.
--
-- A row here means: 'this (chunk, signature) pair was attempted and failed
-- terminally (body file missing on disk, embed returned wrong dim, embedder
-- erred unrecoverably) — DO NOT re-enqueue it on the next backfill batch.'
--
-- Without this table, the reembed worklist's `NOT EXISTS embeddings` predicate
-- keeps re-selecting any chunk that failed read/embed (since no sidecar row
-- was ever written), and `handle_reembed_backfill` loops on the same rows
-- forever — observed in the wild as 16 orphan chunk_ids generating ~128k
-- 'body read failed; skipping' warns across ~8k batch defers. The handler
-- now writes a row here on terminal failure, and the worklist excludes them.
-- Idempotent: the table is created here, and `chrono::Utc` is already imported.
CREATE TABLE IF NOT EXISTS mem_tree_chunk_reembed_skipped (
    chunk_id               TEXT NOT NULL REFERENCES mem_tree_chunks(id) ON DELETE CASCADE,
    model_signature        TEXT NOT NULL,
    reason                 TEXT NOT NULL,
    skipped_at_ms          INTEGER NOT NULL,
    PRIMARY KEY (chunk_id, model_signature)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_chunk_reembed_skipped_model
    ON mem_tree_chunk_reembed_skipped(model_signature);

-- Phase 2 (#708): per-chunk score rationale for admission debugging.
CREATE TABLE IF NOT EXISTS mem_tree_score (
    chunk_id               TEXT PRIMARY KEY,
    total                  REAL NOT NULL,
    token_count_signal     REAL NOT NULL,
    unique_words_signal    REAL NOT NULL,
    metadata_weight        REAL NOT NULL,
    source_weight          REAL NOT NULL,
    interaction_weight     REAL NOT NULL,
    entity_density         REAL NOT NULL,
    dropped                INTEGER NOT NULL DEFAULT 0,
    reason                 TEXT,
    computed_at_ms         INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_score_total
    ON mem_tree_score(total);
CREATE INDEX IF NOT EXISTS idx_mem_tree_score_dropped
    ON mem_tree_score(dropped);

-- Phase 2 (#708): inverted index entity_id -> node_id for retrieval.
-- is_user (#1365) is set at index time via the Composio identity registry
-- (is_self_identity_any_toolkit). Default 0 so legacy rows read back as
-- non-user until the backfill job re-tags them.
CREATE TABLE IF NOT EXISTS mem_tree_entity_index (
    entity_id              TEXT NOT NULL,
    node_id                TEXT NOT NULL,
    node_kind              TEXT NOT NULL,
    entity_kind            TEXT NOT NULL,
    surface                TEXT NOT NULL,
    score                  REAL NOT NULL,
    timestamp_ms           INTEGER NOT NULL,
    tree_id                TEXT,
    is_user                INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (entity_id, node_id)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_entity_index_entity
    ON mem_tree_entity_index(entity_id);
CREATE INDEX IF NOT EXISTS idx_mem_tree_entity_index_node
    ON mem_tree_entity_index(node_id);
CREATE INDEX IF NOT EXISTS idx_mem_tree_entity_index_timestamp
    ON mem_tree_entity_index(timestamp_ms);

-- Phase 3a (#709): summary trees / bucket-seal.
-- `mem_tree_trees` tracks one tree per scope (source/topic/global).
CREATE TABLE IF NOT EXISTS mem_tree_trees (
    id                     TEXT PRIMARY KEY,
    kind                   TEXT NOT NULL,
    scope                  TEXT NOT NULL,
    root_id                TEXT,
    max_level              INTEGER NOT NULL DEFAULT 0,
    status                 TEXT NOT NULL DEFAULT 'active',
    created_at_ms          INTEGER NOT NULL,
    last_sealed_at_ms      INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_mem_tree_trees_kind_scope
    ON mem_tree_trees(kind, scope);
CREATE INDEX IF NOT EXISTS idx_mem_tree_trees_status
    ON mem_tree_trees(status);

-- `mem_tree_summaries` holds sealed summary nodes. Immutable once written
-- (Phase 3a). `deleted` is reserved for future archive cascades.
CREATE TABLE IF NOT EXISTS mem_tree_summaries (
    id                     TEXT PRIMARY KEY,
    tree_id                TEXT NOT NULL,
    tree_kind              TEXT NOT NULL,
    level                  INTEGER NOT NULL,
    parent_id              TEXT,
    child_ids_json         TEXT NOT NULL DEFAULT '[]',
    content                TEXT NOT NULL,
    token_count            INTEGER NOT NULL,
    entities_json          TEXT NOT NULL DEFAULT '[]',
    topics_json            TEXT NOT NULL DEFAULT '[]',
    time_range_start_ms    INTEGER NOT NULL,
    time_range_end_ms      INTEGER NOT NULL,
    score                  REAL NOT NULL DEFAULT 0.0,
    sealed_at_ms           INTEGER NOT NULL,
    deleted                INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (tree_id) REFERENCES mem_tree_trees(id)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_summaries_tree_level
    ON mem_tree_summaries(tree_id, level);
CREATE INDEX IF NOT EXISTS idx_mem_tree_summaries_parent
    ON mem_tree_summaries(parent_id);
CREATE INDEX IF NOT EXISTS idx_mem_tree_summaries_sealed_at
    ON mem_tree_summaries(sealed_at_ms);
CREATE INDEX IF NOT EXISTS idx_mem_tree_summaries_deleted
    ON mem_tree_summaries(deleted);

-- Per-(summary, embedding model) vectors (#1574). Kept separate from the
-- legacy mem_tree_summaries.embedding column so provider/model switches can
-- be query-time filters instead of destructive rewrites.
CREATE TABLE IF NOT EXISTS mem_tree_summary_embeddings (
    summary_id             TEXT NOT NULL REFERENCES mem_tree_summaries(id) ON DELETE CASCADE,
    model_signature        TEXT NOT NULL,
    vector                 BLOB NOT NULL,
    dim                    INTEGER NOT NULL,
    created_at             REAL NOT NULL,
    PRIMARY KEY (summary_id, model_signature)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_summary_embeddings_model
    ON mem_tree_summary_embeddings(model_signature);

-- #1574 §6 reembed-backfill terminal-skip tombstone (summary side). Mirrors
-- `mem_tree_chunk_reembed_skipped` for the summary worklist. See that table's
-- comment for the full rationale.
CREATE TABLE IF NOT EXISTS mem_tree_summary_reembed_skipped (
    summary_id             TEXT NOT NULL REFERENCES mem_tree_summaries(id) ON DELETE CASCADE,
    model_signature        TEXT NOT NULL,
    reason                 TEXT NOT NULL,
    skipped_at_ms          INTEGER NOT NULL,
    PRIMARY KEY (summary_id, model_signature)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_summary_reembed_skipped_model
    ON mem_tree_summary_reembed_skipped(model_signature);

-- `mem_tree_buffers` holds the unsealed frontier per (tree, level). One row
-- per active level per tree; deleted when the buffer seals (clears) in the
-- same transaction as the new summary node row.
CREATE TABLE IF NOT EXISTS mem_tree_buffers (
    tree_id                TEXT NOT NULL,
    level                  INTEGER NOT NULL,
    item_ids_json          TEXT NOT NULL DEFAULT '[]',
    token_sum              INTEGER NOT NULL DEFAULT 0,
    oldest_at_ms           INTEGER,
    updated_at_ms          INTEGER NOT NULL,
    PRIMARY KEY (tree_id, level),
    FOREIGN KEY (tree_id) REFERENCES mem_tree_trees(id)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_buffers_oldest
    ON mem_tree_buffers(oldest_at_ms);

-- Phase 3c (#709): per-entity hotness counters driving lazy topic-tree
-- materialisation. One row per canonical entity_id. Counters are bumped
-- on every ingest; `last_hotness` is recomputed every
-- `TOPIC_RECHECK_EVERY` ingests to decide whether to spawn / archive a
-- topic tree for the entity. TODO: 30-day windowing — for Phase 3c we
-- increment counts forever and rely on project-scale truthfulness.
CREATE TABLE IF NOT EXISTS mem_tree_entity_hotness (
    entity_id              TEXT PRIMARY KEY,
    mention_count_30d      INTEGER NOT NULL DEFAULT 0,
    distinct_sources       INTEGER NOT NULL DEFAULT 0,
    last_seen_ms           INTEGER,
    query_hits_30d         INTEGER NOT NULL DEFAULT 0,
    graph_centrality       REAL,
    ingests_since_check    INTEGER NOT NULL DEFAULT 0,
    last_hotness           REAL,
    last_updated_ms        INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_entity_hotness_score
    ON mem_tree_entity_hotness(last_hotness);

-- Async job queue for memory-tree work (extract → admit → buffer → seal →
-- topic-route → daily digest). Producers (ingest, schedulers, handlers)
-- enqueue rows transactionally; the worker pool claims them via the
-- `(status, available_at_ms)` index. `dedupe_key` is enforced as unique
-- only for ready/running rows so a completed job's key can be re-used.
CREATE TABLE IF NOT EXISTS mem_tree_jobs (
    id                     TEXT PRIMARY KEY,
    kind                   TEXT NOT NULL,
    payload_json           TEXT NOT NULL,
    dedupe_key             TEXT,
    status                 TEXT NOT NULL DEFAULT 'ready',
    attempts               INTEGER NOT NULL DEFAULT 0,
    max_attempts           INTEGER NOT NULL DEFAULT 5,
    available_at_ms        INTEGER NOT NULL,
    locked_until_ms        INTEGER,
    last_error             TEXT,
    created_at_ms          INTEGER NOT NULL,
    started_at_ms          INTEGER,
    completed_at_ms        INTEGER
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_jobs_ready
    ON mem_tree_jobs(status, available_at_ms);
CREATE INDEX IF NOT EXISTS idx_mem_tree_jobs_kind
    ON mem_tree_jobs(kind);
CREATE UNIQUE INDEX IF NOT EXISTS idx_mem_tree_jobs_dedupe_active
    ON mem_tree_jobs(dedupe_key)
    WHERE dedupe_key IS NOT NULL AND status IN ('ready', 'running');

-- Source-level ingest gate. Memory items (documents, chat batches, email
-- threads) are append-only — once a `(source_kind, source_id)` is ingested
-- it must not be re-ingested, otherwise its chunks flow back through
-- extract → admit → buffer → seal and end up duplicated in the summariser
-- tree. The first ingest claims the row; subsequent ingest_* calls for the
-- same key short-circuit before canonicalisation.
CREATE TABLE IF NOT EXISTS mem_tree_ingested_sources (
    source_kind            TEXT NOT NULL,
    source_id              TEXT NOT NULL,
    ingested_at_ms         INTEGER NOT NULL,
    PRIMARY KEY (source_kind, source_id)
);
";

/// Upsert a batch of chunks atomically.
///
/// Returns the number of rows inserted or replaced. Duplicates on `chunk.id`
/// are replaced, making the operation idempotent for re-ingest of the same
/// raw source.
pub fn upsert_chunks(config: &Config, chunks: &[Chunk]) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }
    log::debug!(
        "[memory::chunk_store] upsert_chunks: n={} first_id={}",
        chunks.len(),
        chunks[0].id
    );
    with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO mem_tree_chunks (
                    id, source_kind, source_id, source_ref, owner,
                    timestamp_ms, time_range_start_ms, time_range_end_ms,
                    tags_json, content, token_count, seq_in_source, created_at_ms
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                ON CONFLICT(id) DO UPDATE SET
                    source_kind = excluded.source_kind,
                    source_id = excluded.source_id,
                    source_ref = excluded.source_ref,
                    owner = excluded.owner,
                    timestamp_ms = excluded.timestamp_ms,
                    time_range_start_ms = excluded.time_range_start_ms,
                    time_range_end_ms = excluded.time_range_end_ms,
                    tags_json = excluded.tags_json,
                    content = excluded.content,
                    token_count = excluded.token_count,
                    seq_in_source = excluded.seq_in_source,
                    created_at_ms = excluded.created_at_ms",
            )?;
            upsert_chunks_with_statement(&mut stmt, chunks)?;
        }
        tx.commit()?;
        Ok(chunks.len())
    })
}

/// Upsert chunks using an existing transaction, preserving previously stored embeddings.
pub(crate) fn upsert_chunks_tx(tx: &Transaction<'_>, chunks: &[Chunk]) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT INTO mem_tree_chunks (
            id, source_kind, source_id, source_ref, owner,
            timestamp_ms, time_range_start_ms, time_range_end_ms,
            tags_json, content, token_count, seq_in_source, created_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
        ON CONFLICT(id) DO UPDATE SET
            source_kind = excluded.source_kind,
            source_id = excluded.source_id,
            source_ref = excluded.source_ref,
            owner = excluded.owner,
            timestamp_ms = excluded.timestamp_ms,
            time_range_start_ms = excluded.time_range_start_ms,
            time_range_end_ms = excluded.time_range_end_ms,
            tags_json = excluded.tags_json,
            content = excluded.content,
            token_count = excluded.token_count,
            seq_in_source = excluded.seq_in_source,
            created_at_ms = excluded.created_at_ms",
    )?;
    upsert_chunks_with_statement(&mut stmt, chunks)?;
    Ok(chunks.len())
}

/// Upsert staged chunks (with content_path + content_sha256) using an existing transaction.
///
/// Identical to `upsert_chunks_tx` but also writes the Phase MD-content pointer columns.
/// `content` column receives a ≤500-char plain-text preview of the body (the full body
/// lives on disk at `content_path`).
pub(crate) fn upsert_staged_chunks_tx(
    tx: &Transaction<'_>,
    staged: &[StagedChunk],
) -> Result<usize> {
    if staged.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT INTO mem_tree_chunks (
            id, source_kind, source_id, source_ref, owner,
            timestamp_ms, time_range_start_ms, time_range_end_ms,
            tags_json, content, token_count, seq_in_source, created_at_ms,
            content_path, content_sha256
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
        ON CONFLICT(id) DO UPDATE SET
            source_kind = excluded.source_kind,
            source_id = excluded.source_id,
            source_ref = excluded.source_ref,
            owner = excluded.owner,
            timestamp_ms = excluded.timestamp_ms,
            time_range_start_ms = excluded.time_range_start_ms,
            time_range_end_ms = excluded.time_range_end_ms,
            tags_json = excluded.tags_json,
            content = excluded.content,
            token_count = excluded.token_count,
            seq_in_source = excluded.seq_in_source,
            created_at_ms = excluded.created_at_ms,
            content_path = excluded.content_path,
            content_sha256 = excluded.content_sha256",
    )?;
    for s in staged {
        let chunk = &s.chunk;
        // SQL `content` column always carries a ≤500-char preview now
        // — the full body either lives at `content_path` (chat /
        // document) or is reconstructed from `raw_refs_json` byte
        // ranges in the raw archive (email). See `read_chunk_body`.
        let preview: String = chunk.content.chars().take(500).collect();
        stmt.execute(params![
            chunk.id,
            chunk.metadata.source_kind.as_str(),
            chunk.metadata.source_id,
            chunk.metadata.source_ref.as_ref().map(|r| r.value.as_str()),
            chunk.metadata.owner,
            chunk.metadata.timestamp.timestamp_millis(),
            chunk.metadata.time_range.0.timestamp_millis(),
            chunk.metadata.time_range.1.timestamp_millis(),
            serde_json::to_string(&chunk.metadata.tags)?,
            preview,
            chunk.token_count,
            chunk.seq_in_source,
            chunk.created_at.timestamp_millis(),
            s.content_path,
            s.content_sha256,
        ])?;
    }
    Ok(staged.len())
}

fn upsert_chunks_with_statement(
    stmt: &mut rusqlite::Statement<'_>,
    chunks: &[Chunk],
) -> Result<()> {
    for chunk in chunks {
        stmt.execute(params![
            chunk.id,
            chunk.metadata.source_kind.as_str(),
            chunk.metadata.source_id,
            chunk.metadata.source_ref.as_ref().map(|r| r.value.as_str()),
            chunk.metadata.owner,
            chunk.metadata.timestamp.timestamp_millis(),
            chunk.metadata.time_range.0.timestamp_millis(),
            chunk.metadata.time_range.1.timestamp_millis(),
            serde_json::to_string(&chunk.metadata.tags)?,
            chunk.content,
            chunk.token_count,
            chunk.seq_in_source,
            chunk.created_at.timestamp_millis(),
        ])?;
    }
    Ok(())
}

/// Fetch one chunk by its id.
pub fn get_chunk(config: &Config, id: &str) -> Result<Option<Chunk>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, source_kind, source_id, source_ref, owner,
                    timestamp_ms, time_range_start_ms, time_range_end_ms,
                    tags_json, content, token_count, seq_in_source, created_at_ms
               FROM mem_tree_chunks WHERE id = ?1",
        )?;
        let row = stmt
            .query_row(params![id], row_to_chunk)
            .optional()
            .context("Failed to query chunk by id")?;
        Ok(row)
    })
}

/// Query parameters for [`list_chunks`]. All fields are optional filters —
/// callers pass `ListChunksQuery::default()` to get recent-across-everything.
#[derive(Debug, Default, Clone)]
pub struct ListChunksQuery {
    pub source_kind: Option<SourceKind>,
    pub source_id: Option<String>,
    pub owner: Option<String>,
    /// Inclusive lower bound on `timestamp` (milliseconds since epoch).
    pub since_ms: Option<i64>,
    /// Inclusive upper bound on `timestamp` (milliseconds since epoch).
    pub until_ms: Option<i64>,
    /// Max rows to return (default 100 when `None`).
    pub limit: Option<usize>,
}

/// List chunks matching the provided filters, ordered by `timestamp` DESC.
pub fn list_chunks(config: &Config, query: &ListChunksQuery) -> Result<Vec<Chunk>> {
    with_connection(config, |conn| {
        let mut sql = String::from(
            "SELECT id, source_kind, source_id, source_ref, owner,
                    timestamp_ms, time_range_start_ms, time_range_end_ms,
                    tags_json, content, token_count, seq_in_source, created_at_ms
               FROM mem_tree_chunks WHERE 1=1",
        );
        let mut bound: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(kind) = query.source_kind {
            sql.push_str(" AND source_kind = ?");
            bound.push(Box::new(kind.as_str().to_string()));
        }
        if let Some(ref source_id) = query.source_id {
            sql.push_str(" AND source_id = ?");
            bound.push(Box::new(source_id.clone()));
        }
        if let Some(ref owner) = query.owner {
            sql.push_str(" AND owner = ?");
            bound.push(Box::new(owner.clone()));
        }
        if let Some(since_ms) = query.since_ms {
            sql.push_str(" AND timestamp_ms >= ?");
            bound.push(Box::new(since_ms));
        }
        if let Some(until_ms) = query.until_ms {
            sql.push_str(" AND timestamp_ms <= ?");
            bound.push(Box::new(until_ms));
        }
        let limit = normalized_limit(query.limit);
        sql.push_str(" ORDER BY timestamp_ms DESC, seq_in_source ASC LIMIT ?");
        bound.push(Box::new(limit));

        let mut stmt = conn.prepare(&sql)?;
        let param_refs: Vec<&dyn rusqlite::ToSql> = bound
            .iter()
            .map(|b| b.as_ref() as &dyn rusqlite::ToSql)
            .collect();
        let rows = stmt
            .query_map(param_refs.as_slice(), row_to_chunk)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect chunks")?;
        Ok(rows)
    })
}

/// Count total chunks in the store (useful for tests / diagnostics).
pub fn count_chunks(config: &Config) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_chunks", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
    })
}

/// Set the lifecycle status column for `chunk_id`. See `CHUNK_STATUS_*`.
pub fn set_chunk_lifecycle_status(config: &Config, chunk_id: &str, status: &str) -> Result<()> {
    with_connection(config, |conn| {
        set_chunk_lifecycle_status_conn(conn, chunk_id, status)
    })
}

pub(crate) fn set_chunk_lifecycle_status_tx(
    tx: &Transaction<'_>,
    chunk_id: &str,
    status: &str,
) -> Result<()> {
    set_chunk_lifecycle_status_conn(tx, chunk_id, status)
}

/// Read the lifecycle status column for `chunk_id`, or `None` if the row is absent.
pub fn get_chunk_lifecycle_status(config: &Config, chunk_id: &str) -> Result<Option<String>> {
    with_connection(config, |conn| {
        get_chunk_lifecycle_status_conn(conn, chunk_id)
    })
}

pub(crate) fn get_chunk_lifecycle_status_tx(
    tx: &Transaction<'_>,
    chunk_id: &str,
) -> Result<Option<String>> {
    get_chunk_lifecycle_status_conn(tx, chunk_id)
}

fn get_chunk_lifecycle_status_conn(conn: &Connection, chunk_id: &str) -> Result<Option<String>> {
    let row = conn
        .query_row(
            "SELECT lifecycle_status FROM mem_tree_chunks WHERE id = ?1",
            params![chunk_id],
            |r| r.get::<_, String>(0),
        )
        .optional()?;
    Ok(row)
}

/// Count chunks currently sitting at a given lifecycle status (test/diagnostic helper).
pub fn count_chunks_by_lifecycle_status(config: &Config, status: &str) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_chunks WHERE lifecycle_status = ?1",
            params![status],
            |r| r.get(0),
        )?;
        Ok(n.max(0) as u64)
    })
}

fn set_chunk_lifecycle_status_conn(conn: &Connection, chunk_id: &str, status: &str) -> Result<()> {
    let changed = conn.execute(
        "UPDATE mem_tree_chunks SET lifecycle_status = ?1 WHERE id = ?2",
        params![status, chunk_id],
    )?;
    if changed == 0 {
        log::warn!(
            "[memory::chunk_store] lifecycle update affected 0 rows chunk_id={} status={}",
            chunk_id,
            status
        );
    }
    Ok(())
}

/// Best-effort, non-transactional check used by `ingest_*` to skip
/// canonicalisation when a source has already been ingested. The
/// authoritative gate is [`claim_source_ingest_tx`] inside the persist
/// transaction — this lookup just avoids burning canonicaliser work on
/// the obvious dup case.
pub fn is_source_ingested(
    config: &Config,
    source_kind: SourceKind,
    source_id: &str,
) -> Result<bool> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM mem_tree_ingested_sources \
             WHERE source_kind = ?1 AND source_id = ?2",
            params![source_kind.as_str(), source_id],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    })
}

/// Atomically claim `(source_kind, source_id)` for ingestion. Returns
/// `true` if the row was newly inserted (caller should proceed with the
/// rest of the persist transaction); `false` if a previous ingest already
/// claimed this source (caller must roll back / skip).
///
/// Lives inside the same transaction as the chunk + job writes so two
/// concurrent ingests of the same source can't both pass the gate.
pub(crate) fn claim_source_ingest_tx(
    tx: &Transaction<'_>,
    source_kind: SourceKind,
    source_id: &str,
    now_ms: i64,
) -> Result<bool> {
    let inserted = tx.execute(
        "INSERT OR IGNORE INTO mem_tree_ingested_sources \
            (source_kind, source_id, ingested_at_ms) \
         VALUES (?1, ?2, ?3)",
        params![source_kind.as_str(), source_id, now_ms],
    )?;
    Ok(inserted > 0)
}

fn row_to_chunk(row: &rusqlite::Row<'_>) -> rusqlite::Result<Chunk> {
    let id: String = row.get(0)?;
    let source_kind_s: String = row.get(1)?;
    let source_id: String = row.get(2)?;
    let source_ref: Option<String> = row.get(3)?;
    let owner: String = row.get(4)?;
    let ts_ms: i64 = row.get(5)?;
    let trs_ms: i64 = row.get(6)?;
    let tre_ms: i64 = row.get(7)?;
    let tags_json: String = row.get(8)?;
    let content: String = row.get(9)?;
    let token_count: i64 = row.get(10)?;
    let seq: i64 = row.get(11)?;
    let created_ms: i64 = row.get(12)?;

    let source_kind = SourceKind::parse(&source_kind_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
    })?;
    let timestamp = ms_to_utc(ts_ms)?;
    let time_range = (ms_to_utc(trs_ms)?, ms_to_utc(tre_ms)?);
    let created_at = ms_to_utc(created_ms)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
    })?;

    Ok(Chunk {
        id,
        content,
        metadata: Metadata {
            source_kind,
            source_id,
            owner,
            timestamp,
            time_range,
            tags,
            source_ref: source_ref.map(SourceRef::new),
        },
        token_count: token_count.max(0) as u32,
        seq_in_source: seq.max(0) as u32,
        created_at,
        // partial_message is not stored in SQLite — it's a transient chunker
        // signal. Chunks read back from DB always get false (the column doesn't
        // exist; callers that need this flag hold the Chunk in memory).
        partial_message: false,
    })
}

fn ms_to_utc(ms: i64) -> rusqlite::Result<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single().ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("invalid timestamp ms {ms}").into(),
        )
    })
}

// ── Schema-apply instrumentation (test-only) ─────────────────────────────────
//
// Per-path counter of how many times `apply_schema` ran for each DB path,
// gated behind `cfg(test)` so the production binary carries no overhead.
// Used by the concurrent-init regression test to assert "exactly once per
// path" across racing workers; it survives even when the connection cache
// is cleared between tests because tests use distinct tempdirs.
#[cfg(test)]
static SCHEMA_APPLY_COUNTS: OnceLock<Mutex<HashMap<PathBuf, usize>>> = OnceLock::new();

fn record_schema_apply(_path: &Path) {
    #[cfg(test)]
    {
        let counts = SCHEMA_APPLY_COUNTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = counts
            .lock()
            .expect("memory_tree schema apply count mutex poisoned");
        *guard.entry(_path.to_path_buf()).or_insert(0) += 1;
    }
}

#[cfg(test)]
#[doc(hidden)]
pub(crate) fn schema_apply_count_for_path_for_tests(path: &Path) -> usize {
    SCHEMA_APPLY_COUNTS
        .get()
        .and_then(|m| {
            m.lock()
                .ok()
                .map(|guard| guard.get(path).copied().unwrap_or(0))
        })
        .unwrap_or(0)
}

// SQLite extended result codes that fire during cold-start WAL/SHM bootstrap
// races. NOTE on values: extended codes are `SQLITE_IOERR (10) | (sub << 8)`.
// 4874 is `IOERR_SHMSIZE` (sub 19), NOT `SHMMAP` — the real `SHMMAP` is 5386
// (sub 21) and the "open a new shared-memory segment" failure is `SHMOPEN`
// 4618 (sub 18), which is what surfaced on macOS. The whole `-shm` family is
// listed so the classifiers don't miss any of them.
/// `CANTOPEN` — racing the lockfile/WAL creation done by another connection.
const SQLITE_CANTOPEN: i32 = 14;
/// `IOERR_TRUNCATE` — the WAL/db is being truncated during bootstrap.
const SQLITE_IOERR_TRUNCATE: i32 = 1546;
/// `IOERR_SHMOPEN` — opening a new `-shm` shared-memory segment failed (the
/// macOS cold-start failure, e.g. Sentry TAURI-RUST-X1).
const SQLITE_IOERR_SHMOPEN: i32 = 4618;
/// `IOERR_SHMSIZE` — the `-shm` file is being resized during bootstrap.
const SQLITE_IOERR_SHMSIZE: i32 = 4874;
/// `IOERR_SHMMAP` — mapping a page of the `-shm` wal-index failed.
const SQLITE_IOERR_SHMMAP: i32 = 5386;
/// `IOERR_IN_PAGE` — an mmap-page I/O fault, also seen under WAL cold-start.
const SQLITE_IOERR_IN_PAGE: i32 = 8714;

/// True if `err` (or anything in its cause chain) is one of the SQLite codes
/// that fire during cold-start WAL/SHM bootstrap races: `CANTOPEN`,
/// `IOERR_TRUNCATE`, the `-shm` family (`SHMOPEN` / `SHMSIZE` / `SHMMAP`), and
/// `IOERR_IN_PAGE`.
pub(crate) fn is_transient_cold_start(err: &anyhow::Error) -> bool {
    fn is_transient_sqlite(e: &(dyn std::error::Error + 'static)) -> bool {
        if let Some(rusqlite::Error::SqliteFailure(ffi, _)) = e.downcast_ref::<rusqlite::Error>() {
            return matches!(
                ffi.extended_code,
                SQLITE_CANTOPEN
                    | SQLITE_IOERR_TRUNCATE
                    | SQLITE_IOERR_SHMOPEN
                    | SQLITE_IOERR_SHMSIZE
                    | SQLITE_IOERR_SHMMAP
                    | SQLITE_IOERR_IN_PAGE
            );
        }
        false
    }
    if is_transient_sqlite(err.root_cause()) {
        return true;
    }
    let mut src: Option<&(dyn std::error::Error + 'static)> = Some(err.as_ref());
    while let Some(cur) = src {
        if is_transient_sqlite(cur) {
            return true;
        }
        src = cur.source();
    }
    false
}

// ── Connection cache (#2206) ─────────────────────────────────────────────────

/// How many consecutive init failures before the circuit breaker trips.
const CB_THRESHOLD: u32 = 3;
/// How long the circuit breaker holds the DB closed after tripping.
const CB_COOLDOWN: Duration = Duration::from_secs(30);

/// Per-path circuit breaker: after [`CB_THRESHOLD`] consecutive init failures
/// the breaker trips and `get_or_init_connection` returns an error immediately
/// until [`CB_COOLDOWN`] elapses. On the first success it resets to zero.
struct CircuitBreaker {
    consecutive_failures: AtomicU32,
    tripped: AtomicBool,
    last_trip: PMutex<Option<Instant>>,
}

impl CircuitBreaker {
    fn new() -> Self {
        Self {
            consecutive_failures: AtomicU32::new(0),
            tripped: AtomicBool::new(false),
            last_trip: PMutex::new(None),
        }
    }

    fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
        self.tripped.store(false, Ordering::Relaxed);
        *self.last_trip.lock() = None;
    }

    /// Records one more failure. Returns `true` if this call just tripped the
    /// breaker (i.e. the threshold was crossed right now).
    fn record_failure(&self) -> bool {
        let prev = self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
        let count = prev + 1;
        if count >= CB_THRESHOLD && !self.tripped.swap(true, Ordering::Relaxed) {
            *self.last_trip.lock() = Some(Instant::now());
            return true;
        }
        // Re-arm the cooldown on each subsequent failure while already tripped
        // so that a failed post-cooldown retry restarts the 30 s window instead
        // of leaving the stale timestamp in place (which would let `is_open`
        // return false immediately and allow unlimited retries).
        if self.tripped.load(Ordering::Relaxed) {
            *self.last_trip.lock() = Some(Instant::now());
        }
        false
    }

    /// Returns `true` when the breaker is open AND the cooldown has not yet
    /// elapsed. Returns `false` (allowing a retry) once the cooldown passes.
    fn is_open(&self) -> bool {
        if !self.tripped.load(Ordering::Relaxed) {
            return false;
        }
        let guard = self.last_trip.lock();
        match *guard {
            Some(t) if t.elapsed() < CB_COOLDOWN => true,
            _ => false,
        }
    }
}

/// Process-level cache — two separate maps so that a failing init cannot
/// accidentally serve a dummy connection on the fast path.
///
/// `connections`: only fully-initialised (schema + migrations run) entries.
/// `breakers`:    persists across failed init attempts so the circuit breaker
///                survives even when `connections` has no entry for that path.
struct ConnectionCache {
    connections: PMutex<HashMap<PathBuf, Arc<PMutex<Connection>>>>,
    breakers: PMutex<HashMap<PathBuf, Arc<CircuitBreaker>>>,
    /// Per-path mutex held across the slow-path init so concurrent
    /// workers racing into `with_connection` on a cold DB serialise on
    /// the WAL+SHM bootstrap. Without this, N threads see "no cached
    /// connection" simultaneously and all run `apply_schema`, which is
    /// idempotent but reopens the cold-start race window
    /// (OPENHUMAN-TAURI-HH / -ZM / -MB).
    init_locks: PMutex<HashMap<PathBuf, Arc<PMutex<()>>>>,
}

static CONN_CACHE: OnceLock<ConnectionCache> = OnceLock::new();

fn conn_cache() -> &'static ConnectionCache {
    CONN_CACHE.get_or_init(|| ConnectionCache {
        connections: PMutex::new(HashMap::new()),
        breakers: PMutex::new(HashMap::new()),
        init_locks: PMutex::new(HashMap::new()),
    })
}

/// Compute the canonical DB path from `config`.
fn db_path_for(config: &Config) -> PathBuf {
    config.workspace_dir.join(DB_DIR).join(DB_FILE)
}

/// Delete stale WAL/SHM side-files (`<db>-shm`, `<db>-wal`) that can block a
/// clean DB open after a crash. Logs what was deleted and returns `true` if
/// anything was removed.
///
/// SQLite names these files `<db_path>-shm` and `<db_path>-wal`.
/// For `chunks.db` that is `chunks.db-shm` / `chunks.db-wal`.
pub(crate) fn try_cleanup_stale_files(db_path: &std::path::Path) -> bool {
    let mut cleaned = false;
    for suffix in &["-shm", "-wal"] {
        // Build the side-file path: append suffix to the full db filename.
        let side = {
            let mut p = db_path.to_path_buf();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            p.set_file_name(format!("{name}{suffix}"));
            p
        };
        if side.exists() {
            match std::fs::remove_file(&side) {
                Ok(()) => {
                    log::warn!("[memory_tree] removed stale side-file: {}", side.display());
                    cleaned = true;
                }
                Err(e) => {
                    log::warn!(
                        "[memory_tree] failed to remove stale side-file {}: {e}",
                        side.display()
                    );
                }
            }
        }
    }
    cleaned
}

/// Run the full one-time DB initialisation (journal mode, schema, migrations)
/// against an already-open `Connection`. Used by `get_or_init_connection`.
fn init_db(conn: &Connection, config: &Config) -> Result<()> {
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)
        .context("Failed to configure memory_tree busy timeout")?;
    // SQLite resets `foreign_keys` to off on every new connection. The
    // ConnectionCache holds one cached `Connection` per DB path, so
    // setting it here (alongside the rest of init) is the per-connection
    // surface — fast-path callers reuse the cached conn with FKs already
    // on.
    conn.execute_batch("PRAGMA foreign_keys = ON;")
        .context("Failed to enable memory_tree foreign_keys pragma")?;
    // memory_tree runs the TRUNCATE rollback journal (see `apply_schema`), so
    // crash-safety requires synchronous=FULL — NORMAL is only corruption-safe
    // under WAL. Set explicitly so a future global default can't weaken it.
    conn.execute_batch("PRAGMA synchronous = FULL;")
        .context("Failed to set memory_tree synchronous=FULL")?;
    apply_schema(conn)?;
    // #1574 §7: one-shot, version-gated legacy→sidecar embedding migration.
    migrate_legacy_embeddings_to_sidecar(conn, config)?;
    Ok(())
}

fn apply_schema(conn: &Connection) -> Result<()> {
    // Note: `init_db` runs the `#1574 §7` legacy→sidecar embedding migration
    // after this returns, so the dim-equal copy step is not duplicated here.
    // memory_tree uses the TRUNCATE rollback journal, NOT WAL. WAL's `-shm`
    // shared-memory index + `-wal` checkpoint machinery are the root of the
    // cold-start IOERR_SHMMAP (macOS) / IOERR_TRUNCATE (Windows, AV-held
    // handles) failures (Sentry TAURI-RUST-EV / TAURI-RUST-X1). All tree
    // access serialises on the single cached `PMutex<Connection>` (see
    // `get_or_init_connection`), so WAL's only real benefit — concurrent
    // readers — is unused here, which makes WAL pure liability. The sibling
    // tree DBs (cron / vault / redirect_links) already run the default
    // rollback journal without issue.
    //
    // Requesting TRUNCATE on a database a prior release left in WAL mode
    // checkpoints the `-wal` back into the main file and removes the
    // `-wal`/`-shm` side-files, so this also migrates existing WAL databases
    // in place on upgrade.
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode=TRUNCATE", [], |row| row.get(0))
        .context("Failed to set memory_tree journal_mode=TRUNCATE")?;
    if !journal_mode.eq_ignore_ascii_case("truncate") {
        log::warn!(
            "[memory_tree] journal_mode is '{journal_mode}' after requesting TRUNCATE \
             — a prior WAL connection or a locked -wal may be blocking the switch"
        );
    }
    conn.execute_batch(SCHEMA)
        .context("Failed to initialize memory_tree schema")?;
    // Phase 2 migrations — additive, idempotent.
    add_column_if_missing(conn, "mem_tree_chunks", "embedding", "BLOB")?;
    // Phase 2 LLM-NER follow-up: per-chunk LLM importance signal +
    // human-readable reason. Both nullable; absence is treated as
    // "no LLM signal available" by readers.
    add_column_if_missing(conn, "mem_tree_score", "llm_importance", "REAL")?;
    add_column_if_missing(conn, "mem_tree_score", "llm_importance_reason", "TEXT")?;
    // Phase 3a (#709): parent-summary backlink on leaves.
    add_column_if_missing(conn, "mem_tree_chunks", "parent_summary_id", "TEXT")?;
    // Phase 4 (#710): sealed-summary embeddings for semantic rerank.
    add_column_if_missing(conn, "mem_tree_summaries", "embedding", "BLOB")?;
    // Async-pipeline lifecycle flag. Default 'admitted' so chunks ingested
    // before the queue migration stay queryable.
    add_column_if_missing(
        conn,
        "mem_tree_chunks",
        "lifecycle_status",
        "TEXT NOT NULL DEFAULT 'admitted'",
    )?;
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_mem_tree_chunks_lifecycle \
         ON mem_tree_chunks(lifecycle_status);",
    )
    .context("Failed to create mem_tree_chunks lifecycle index")?;
    // Phase MD-content (#TBD): pointer + integrity hash.
    add_column_if_missing(conn, "mem_tree_chunks", "content_path", "TEXT")?;
    add_column_if_missing(conn, "mem_tree_chunks", "content_sha256", "TEXT")?;
    // Phase MD-content (summaries).
    add_column_if_missing(conn, "mem_tree_summaries", "content_path", "TEXT")?;
    add_column_if_missing(conn, "mem_tree_summaries", "content_sha256", "TEXT")?;
    // Raw-archive pointer column.
    add_column_if_missing(conn, "mem_tree_chunks", "raw_refs_json", "TEXT")?;
    // #1365: is_user flag on indexed entity rows.
    add_column_if_missing(
        conn,
        "mem_tree_entity_index",
        "is_user",
        "INTEGER NOT NULL DEFAULT 0",
    )?;
    Ok(())
}

/// Whether `err` looks like one of the I/O error codes that warrant a
/// stale-file cleanup + single retry before giving up.
fn is_io_open_error(err: &anyhow::Error) -> bool {
    if let Some(rusqlite::Error::SqliteFailure(f, _)) = err.downcast_ref::<rusqlite::Error>() {
        return matches!(
            f.extended_code,
            SQLITE_CANTOPEN
                | SQLITE_IOERR_TRUNCATE
                | SQLITE_IOERR_SHMOPEN
                | SQLITE_IOERR_SHMSIZE
                | SQLITE_IOERR_SHMMAP
                | SQLITE_IOERR_IN_PAGE
        ) || f.code == rusqlite::ErrorCode::CannotOpen;
    }
    let msg = format!("{err:#}").to_ascii_lowercase();
    msg.contains("disk i/o error")
        || msg.contains("unable to open database file")
        || msg.contains("xshmmap")
        || msg.contains("truncate file")
}

/// Obtain (or lazily create) a cached connection for the workspace described
/// by `config`. Returns `Err` immediately when the circuit breaker is open.
fn get_or_init_connection(config: &Config) -> Result<Arc<PMutex<Connection>>> {
    let db_path = db_path_for(config);

    // ── Fast path: reject immediately if the breaker is open ─────────────
    {
        let breakers = conn_cache().breakers.lock();
        if let Some(breaker) = breakers.get(&db_path) {
            if breaker.is_open() {
                anyhow::bail!(
                    "[memory_tree] circuit breaker open for {}: too many consecutive init failures",
                    db_path.display()
                );
            }
        }
    }
    // ── Fast path: return cached connection if already initialised ────────
    {
        let guard = conn_cache().connections.lock();
        if let Some(conn) = guard.get(&db_path) {
            return Ok(Arc::clone(conn));
        }
    }

    // ── Slow path: serialise init per-path so concurrent workers don't
    //    all race into `open_and_init` on a cold DB.
    let init_lock = {
        let mut guard = conn_cache().init_locks.lock();
        guard
            .entry(db_path.clone())
            .or_insert_with(|| Arc::new(PMutex::new(())))
            .clone()
    };
    let _init_guard = init_lock.lock();

    // Re-check the cache once we hold the init lock — another thread
    // may have completed init while we were queued.
    {
        let guard = conn_cache().connections.lock();
        if let Some(conn) = guard.get(&db_path) {
            return Ok(Arc::clone(conn));
        }
    }

    log::debug!(
        "[memory_tree] opening and initialising DB at {}",
        db_path.display()
    );

    // Attempt to open + init the connection (dir creation is inside
    // `open_and_init` so every failure — including EEXIST on the dir —
    // reaches the circuit-breaker recording logic below). On certain I/O
    // errors (#2206) we clean up stale WAL/SHM side-files and retry once.
    let conn = open_and_init(&db_path, config).or_else(|first_err| {
        if is_io_open_error(&first_err) {
            log::warn!(
                "[memory_tree] I/O error on first open attempt ({}), cleaning stale files and retrying",
                first_err
            );
            try_cleanup_stale_files(&db_path);
            open_and_init(&db_path, config)
        } else {
            Err(first_err)
        }
    });

    match conn {
        Ok(conn) => {
            let arc_conn = Arc::new(PMutex::new(conn));
            conn_cache()
                .connections
                .lock()
                .insert(db_path.clone(), Arc::clone(&arc_conn));
            // Reset any prior failure counter now that init succeeded.
            if let Some(breaker) = conn_cache().breakers.lock().get(&db_path) {
                breaker.record_success();
            }
            log::debug!("[memory_tree] DB connection cached and ready");
            Ok(arc_conn)
        }
        Err(err) => {
            // Persist the breaker so the failure count accumulates across
            // calls even though no connection entry exists yet.
            let breaker = {
                let mut guard = conn_cache().breakers.lock();
                guard
                    .entry(db_path.clone())
                    .or_insert_with(|| Arc::new(CircuitBreaker::new()))
                    .clone()
            };
            let just_tripped = breaker.record_failure();
            if just_tripped {
                log::error!(
                    "[memory_tree] circuit breaker tripped for {}: {} consecutive init failures",
                    db_path.display(),
                    CB_THRESHOLD
                );
                let _ = crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::HealthChanged {
                        component: "memory_tree_db".to_string(),
                        healthy: false,
                        message: Some(format!(
                            "Schema init failed {CB_THRESHOLD} consecutive times"
                        )),
                    },
                );
            }
            Err(err)
        }
    }
}

/// Ensure the DB directory exists, open the SQLite file, and run the full
/// schema init sequence. All errors (dir creation, file open, schema init)
/// are returned as `Err` so callers can funnel them through the circuit
/// breaker logic in a single place.
fn open_and_init(db_path: &std::path::Path, config: &Config) -> Result<Connection> {
    let dir = db_path.parent().expect("db_path always has a parent");
    std::fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create memory_tree dir: {}", dir.display()))?;
    let conn = Connection::open(db_path)
        .with_context(|| format!("Failed to open memory_tree DB: {}", db_path.display()))?;
    init_db(&conn, config)
        .with_context(|| format!("Failed to init memory_tree schema: {}", db_path.display()))?;
    record_schema_apply(db_path);
    Ok(conn)
}

/// Remove the cached connection for `config`'s workspace (forces a fresh open
/// on the next `with_connection` call). Also clears the breaker so the next
/// open attempt is not immediately rejected. Does nothing if no entry exists.
#[allow(dead_code)]
pub(crate) fn invalidate_connection(config: &Config) {
    let db_path = db_path_for(config);
    conn_cache().connections.lock().remove(&db_path);
    conn_cache().breakers.lock().remove(&db_path);
    log::debug!(
        "[memory_tree] connection invalidated for {}",
        db_path.display()
    );
}

/// Clear the entire connection cache. For test isolation only.
#[cfg(test)]
pub(crate) fn clear_connection_cache() {
    conn_cache().connections.lock().clear();
    conn_cache().breakers.lock().clear();
    conn_cache().init_locks.lock().clear();
}

/// Open the memory_tree SQLite DB and run a closure against it.
///
/// Visible to sibling modules (e.g. `score::store`) so Phase 2 can reuse
/// the same connection setup / schema initialisation without duplication.
///
/// # Connection caching (#2206)
///
/// The underlying connection is initialised once per workspace path and then
/// reused from a process-level cache. Schema migrations run exactly once on
/// the first call for a given `config.workspace_dir`. Subsequent calls pay
/// only the cost of a `parking_lot::Mutex` lock and the closure itself.
///
/// `#[doc(hidden)] pub` (not `pub(crate)`) because the
/// `memory-tree-init-smoke` bin in `src/bin/` is a separate crate target
/// and must reach this entry point. It is NOT a stable API surface —
/// downstream crates should treat it as internal.
#[doc(hidden)]
pub fn with_connection<T>(config: &Config, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    let conn_arc = get_or_init_connection(config)?;
    let guard = conn_arc.lock();
    f(&guard)
}

/// One-shot migration (#1574 §7, vN): copy legacy `mem_tree_chunks.embedding`
/// / `mem_tree_summaries.embedding` blobs into the per-model sidecar tables
/// under the **active** signature, when (and only when) the legacy vector's
/// dimensionality matches the active embedder's.
///
/// Version-gated via `PRAGMA user_version`: returns immediately once
/// `>= TREE_EMBEDDING_MIGRATION_VERSION`, so the per-open cost is a single
/// pragma read. Dim-mismatched rows are left for the §6 re-embed backfill —
/// the blob's signature is unrecoverable (see spec §7b), so a same-length
/// copy under the active signature is the only provably-safe move and
/// anything else must be re-embedded. The legacy columns are **kept** (read
/// here, dropped only in a later release — spec §7c). Idempotent: re-running
/// before the version bumps re-copies the same rows harmlessly (sidecar
/// upsert is ON CONFLICT); after the bump it is skipped entirely.
fn migrate_legacy_embeddings_to_sidecar(conn: &Connection, config: &Config) -> Result<()> {
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .context("read PRAGMA user_version for #1574 migration")?;
    if version >= TREE_EMBEDDING_MIGRATION_VERSION {
        return Ok(());
    }

    let (provider, model, dims) = crate::openhuman::memory_store::effective_embedding_settings(
        &config.memory,
        config.workload_local_model("embeddings").as_deref(),
    );
    let sig = crate::openhuman::embeddings::format_embedding_signature(&provider, &model, dims);
    log::info!(
        "[memory_tree::migrate] #1574 §7: copying legacy embeddings → sidecar at sig={sig} (dims={dims})"
    );

    let tx = conn.unchecked_transaction()?;
    let mut copied_chunks = 0usize;
    let mut copied_summaries = 0usize;
    let mut skipped_dim_mismatch = 0usize;

    for (table, is_chunk) in [("mem_tree_chunks", true), ("mem_tree_summaries", false)] {
        let mut stmt = tx.prepare(&format!(
            "SELECT id, embedding FROM {table} WHERE embedding IS NOT NULL"
        ))?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
        })?;
        for row in rows {
            let (id, blob) = row?;
            if !blob.len().is_multiple_of(4) {
                log::warn!(
                    "[memory_tree::migrate] {table} id={id}: legacy blob len {} not /4, skipping",
                    blob.len()
                );
                continue;
            }
            if blob.len() / 4 != dims {
                // Different embedding space — unrecoverable from the blob.
                // Leave for the §6 re-embed backfill.
                skipped_dim_mismatch += 1;
                continue;
            }
            let vec: Vec<f32> = blob
                .chunks_exact(4)
                .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                .collect();
            if is_chunk {
                set_chunk_embedding_for_signature_tx(&tx, &id, &sig, &vec)?;
                copied_chunks += 1;
            } else {
                crate::openhuman::memory_store::trees::store::set_summary_embedding_for_signature_tx(
                    &tx, &id, &sig, &vec,
                )?;
                copied_summaries += 1;
            }
        }
    }

    // #1574 §6: enqueue the re-embed backfill ONLY if there is genuinely
    // uncovered work at the active signature (the dim-mismatch slice, or
    // content-bearing rows with no vector). Gating this avoids queuing a
    // no-op job on every DB open — which would otherwise pollute the jobs
    // table for unrelated callers/tests. Enqueued atomically with the
    // migration; dedupe key = signature, so exactly one chain per space.
    let has_uncovered = has_uncovered_reembed_work(&*tx, &sig)?;
    if has_uncovered {
        let backfill_job = crate::openhuman::memory::jobs::types::NewJob::reembed_backfill(
            &crate::openhuman::memory::jobs::types::ReembedBackfillPayload {
                signature: sig.clone(),
            },
        )?;
        crate::openhuman::memory::jobs::enqueue_tx(&tx, &backfill_job)?;
    }

    tx.commit()?;
    conn.pragma_update(None, "user_version", TREE_EMBEDDING_MIGRATION_VERSION)
        .context("set PRAGMA user_version after #1574 migration")?;
    if has_uncovered {
        crate::openhuman::memory::jobs::set_backfill_in_progress(true);
    }
    log::info!(
        "[memory_tree::migrate] #1574 §7 done: copied chunks={copied_chunks} summaries={copied_summaries} \
         skipped_dim_mismatch={skipped_dim_mismatch} (left for §6 re-embed); user_version={TREE_EMBEDDING_MIGRATION_VERSION}"
    );
    Ok(())
}

/// One pointer into the raw archive. A chunk's body is reconstructed by
/// reading each [`RawRef`] in order and joining with `"\n\n"`.
///
/// `start` / `end` are byte offsets into the raw `.md` file. `end =
/// None` means "read to end of file". Both default to "the whole
/// file" (`start = 0`, `end = None`) for the common one-message-one-chunk
/// path; oversize-message chunks get explicit ranges so each chunk
/// reconstructs its sub-slice.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RawRef {
    /// Forward-slash relative path under `<content_root>/`,
    /// e.g. `"raw/gmail-stevent95-at-gmail-dot-com/1700000_msg-id.md"`.
    pub path: String,
    #[serde(default)]
    pub start: usize,
    #[serde(default)]
    pub end: Option<usize>,
}

/// Stash a list of [`RawRef`] entries on a chunk row. Replaces any
/// previous value. Used by ingest pipelines that mirror their bytes
/// into `<content_root>/raw/...` so reads can skip the SQL preview
/// path and pull the full body straight from the archive.
pub fn set_chunk_raw_refs(config: &Config, chunk_id: &str, refs: &[RawRef]) -> Result<()> {
    let json = serde_json::to_string(refs).context("serialize raw_refs")?;
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE mem_tree_chunks SET raw_refs_json = ?1 WHERE id = ?2",
            params![json, chunk_id],
        )?;
        Ok(())
    })
}

/// Return the raw-archive pointers stored in SQLite for `chunk_id`,
/// or `None` if no `raw_refs_json` was recorded.
pub fn get_chunk_raw_refs(config: &Config, chunk_id: &str) -> Result<Option<Vec<RawRef>>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT raw_refs_json FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        match row {
            Some(json) if !json.is_empty() => {
                let refs: Vec<RawRef> =
                    serde_json::from_str(&json).context("deserialize raw_refs_json")?;
                Ok(Some(refs))
            }
            _ => Ok(None),
        }
    })
}

/// Return both `content_path` and `content_sha256` stored in SQLite for `chunk_id`.
///
/// Returns `Ok(None)` if the chunk does not exist or has no content_path recorded yet.
pub fn get_chunk_content_pointers(
    config: &Config,
    chunk_id: &str,
) -> Result<Option<(String, String)>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT content_path, content_sha256 FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
                |r| {
                    let path: Option<String> = r.get(0)?;
                    let sha: Option<String> = r.get(1)?;
                    Ok((path, sha))
                },
            )
            .optional()?;
        Ok(row.and_then(|(p, s)| p.zip(s)))
    })
}

/// Return the `content_path` stored in SQLite for `chunk_id`, if any.
pub fn get_chunk_content_path(config: &Config, chunk_id: &str) -> Result<Option<String>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT content_path FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()?
            .flatten();
        Ok(row)
    })
}

/// Return both `content_path` and `content_sha256` stored in SQLite for `summary_id`.
///
/// Returns `Ok(None)` if the summary does not exist or has no content_path recorded yet
/// (legacy rows pre-MD-content migration).
pub fn get_summary_content_pointers(
    config: &Config,
    summary_id: &str,
) -> Result<Option<(String, String)>> {
    with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT content_path, content_sha256 FROM mem_tree_summaries WHERE id = ?1",
                params![summary_id],
                |r| {
                    let path: Option<String> = r.get(0)?;
                    let sha: Option<String> = r.get(1)?;
                    Ok((path, sha))
                },
            )
            .optional()?;
        Ok(row.and_then(|(p, s)| p.zip(s)))
    })
}

/// List all summary rows that have a non-NULL `content_path`. Used by the
/// bin integrity checker.
pub fn list_summaries_with_content_path(config: &Config) -> Result<Vec<(String, String, String)>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, content_path, content_sha256
               FROM mem_tree_summaries
              WHERE content_path IS NOT NULL AND content_sha256 IS NOT NULL
                AND deleted = 0",
        )?;
        let rows = stmt
            .query_map([], |r| {
                let id: String = r.get(0)?;
                let path: String = r.get(1)?;
                let sha: String = r.get(2)?;
                Ok((id, path, sha))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to list summaries with content_path")?;
        Ok(rows)
    })
}

fn normalized_limit(requested: Option<usize>) -> i64 {
    let clamped = requested
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    i64::try_from(clamped).unwrap_or(MAX_LIST_LIMIT as i64)
}

/// Idempotent `ALTER TABLE ADD COLUMN` — treats an existing column as success.
fn add_column_if_missing(conn: &Connection, table: &str, name: &str, sql_type: &str) -> Result<()> {
    match conn.execute(
        &format!("ALTER TABLE {table} ADD COLUMN {name} {sql_type}"),
        [],
    ) {
        Ok(_) => {
            log::debug!(
                "[memory::chunk_store] migration: added column {table}.{name} ({sql_type})"
            );
            Ok(())
        }
        Err(err) if err.to_string().contains("duplicate column name") => Ok(()),
        Err(err) => Err(err).with_context(|| format!("Failed to add column {table}.{name}")),
    }
}

// ── Phase 2: embedding column accessors ─────────────────────────────────

/// Resolve the active embedding signature for the memory tree from the global
/// [`Config`] — the canonical key every per-model sidecar read/write is scoped
/// by (#1574). Reuses the established local-AI workload derivation
/// ([`Config::workload_local_model`]) and the probe-stable
/// `active_embedding_signature`; introduces no parallel resolution path.
/// `pub(crate)` so the sibling `tree` summary store shares the exact
/// same resolution.
pub(crate) fn tree_active_signature(config: &Config) -> String {
    let local_model = config.workload_local_model("embeddings");
    crate::openhuman::memory_store::active_embedding_signature(
        &config.memory,
        local_model.as_deref(),
    )
}

/// Store a chunk's embedding under the active model signature.
///
/// #1574 cutover: this now writes the per-model `mem_tree_chunk_embeddings`
/// sidecar (via [`set_chunk_embedding_for_signature`]) instead of the legacy
/// `mem_tree_chunks.embedding` column. Call sites are unchanged — the signature
/// is resolved internally from `config`. The legacy column is left intact for
/// the §7 one-shot migration to read; it is dropped only in a later release.
pub fn set_chunk_embedding(config: &Config, chunk_id: &str, embedding: &[f32]) -> Result<()> {
    let signature = tree_active_signature(config);
    log::debug!(
        "[memory::chunk_store] set_chunk_embedding: chunk_id={chunk_id} sig={signature} dims={}",
        embedding.len()
    );
    set_chunk_embedding_for_signature(config, chunk_id, &signature, embedding)
}

/// Core upsert into `mem_tree_chunk_embeddings` over an arbitrary
/// `&Connection`. Shared by the standalone ([`set_chunk_embedding_for_signature`])
/// and in-transaction ([`set_chunk_embedding_for_signature_tx`]) write paths so
/// the SQL exists exactly once. `rusqlite::Transaction` derefs to `Connection`,
/// so an in-tx caller passes `&tx` and the sidecar row commits atomically with
/// the surrounding work (#1574 write-side cutover).
fn upsert_chunk_embedding_conn(
    conn: &rusqlite::Connection,
    chunk_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    let bytes = embedding_to_blob(embedding);
    let dim = i64::try_from(embedding.len()).context("embedding dimension does not fit i64")?;
    let created_at = Utc::now().timestamp_millis() as f64 / 1000.0;
    conn.execute(
        "INSERT INTO mem_tree_chunk_embeddings
             (chunk_id, model_signature, vector, dim, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(chunk_id, model_signature) DO UPDATE SET
                vector = excluded.vector,
                dim = excluded.dim,
                created_at = excluded.created_at",
        rusqlite::params![chunk_id, model_signature, bytes, dim, created_at],
    )?;
    Ok(())
}

/// Store a chunk embedding for a specific provider/model/dimension signature.
///
/// Per-model table write path for #1574. The legacy
/// `mem_tree_chunks.embedding` column is intentionally left untouched by this
/// helper (read by the §7 migration; dropped only in a later release).
pub fn set_chunk_embedding_for_signature(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    with_connection(config, |conn| {
        upsert_chunk_embedding_conn(conn, chunk_id, model_signature, embedding)
    })
}

/// `true` when at least one chunk or summary still needs an embedding at
/// `model_signature` and is not tombstoned as terminally unembeddable.
///
/// Shared by `ensure_reembed_backfill`, the §7 migration enqueue probe, and
/// tests so the worklist and coverage probes cannot drift (#2358).
pub(crate) fn has_uncovered_reembed_work(
    conn: &Connection,
    model_signature: &str,
) -> rusqlite::Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM mem_tree_chunks c
              WHERE NOT EXISTS (SELECT 1 FROM mem_tree_chunk_embeddings e
                                 WHERE e.chunk_id = c.id AND e.model_signature = ?1)
                AND NOT EXISTS (SELECT 1 FROM mem_tree_chunk_reembed_skipped sk
                                 WHERE sk.chunk_id = c.id AND sk.model_signature = ?1))
           OR EXISTS(
             SELECT 1 FROM mem_tree_summaries s
              WHERE s.deleted = 0
                AND NOT EXISTS (SELECT 1 FROM mem_tree_summary_embeddings e
                                 WHERE e.summary_id = s.id AND e.model_signature = ?1)
                AND NOT EXISTS (SELECT 1 FROM mem_tree_summary_reembed_skipped sk
                                 WHERE sk.summary_id = s.id AND sk.model_signature = ?1))",
        rusqlite::params![model_signature],
        |r| r.get(0),
    )
}

/// Persistently record that `(chunk_id, signature)` cannot be re-embedded.
///
/// Called by `handle_reembed_backfill` when the per-chunk body file is
/// missing on disk (orphan) or the embedder rejects the row terminally
/// (wrong dim / unrecoverable embed error). Inserting a row here causes
/// the next backfill batch's worklist query to exclude this chunk via the
/// `NOT EXISTS … mem_tree_chunk_reembed_skipped …` predicate, so the
/// runaway "skipping" loop terminates instead of revisiting the same row
/// every 5 s forever (#1574 §6 fix).
pub fn mark_chunk_reembed_skipped(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
    reason: &str,
) -> Result<()> {
    let chunk_id = validate_reembed_skip_key("chunk_id", chunk_id)?;
    let model_signature = validate_reembed_skip_key("model_signature", model_signature)?;
    with_connection(config, |conn| {
        let now_ms = Utc::now().timestamp_millis();
        conn.execute(
            "INSERT INTO mem_tree_chunk_reembed_skipped
                 (chunk_id, model_signature, reason, skipped_at_ms)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(chunk_id, model_signature) DO UPDATE SET
                    reason = excluded.reason,
                    skipped_at_ms = excluded.skipped_at_ms",
            rusqlite::params![chunk_id, model_signature, reason, now_ms],
        )?;
        log::debug!(
            "[memory::chunk_store] mark_chunk_reembed_skipped chunk_id={chunk_id} sig={model_signature} reason={reason}"
        );
        Ok(())
    })
}

/// Remove a single chunk tombstone so re-embed backfill can retry the row.
///
/// Idempotent: deleting a missing `(chunk_id, model_signature)` pair is a
/// no-op. Intended for operator recovery after environmental failures (moved
/// workspace, restored body files, fixed embedder config) — see #2358.
pub fn clear_chunk_reembed_skipped(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
) -> Result<()> {
    let chunk_id = validate_reembed_skip_key("chunk_id", chunk_id)?;
    let model_signature = validate_reembed_skip_key("model_signature", model_signature)?;
    with_connection(config, |conn| {
        conn.execute(
            "DELETE FROM mem_tree_chunk_reembed_skipped
              WHERE chunk_id = ?1 AND model_signature = ?2",
            rusqlite::params![chunk_id, model_signature],
        )?;
        log::debug!(
            "[memory::chunk_store] clear_chunk_reembed_skipped chunk_id={chunk_id} sig={model_signature}"
        );
        Ok(())
    })
}

/// Clear all chunk and summary tombstones for a model signature.
///
/// Returns the total number of rows removed across both tombstone tables.
/// Idempotent when no tombstones exist for the signature.
pub fn clear_reembed_skipped_for_signature(
    config: &Config,
    model_signature: &str,
) -> Result<usize> {
    let model_signature = validate_reembed_skip_key("model_signature", model_signature)?;
    with_connection(config, |conn| {
        let chunk_deleted = conn.execute(
            "DELETE FROM mem_tree_chunk_reembed_skipped WHERE model_signature = ?1",
            rusqlite::params![model_signature],
        )?;
        let summary_deleted = conn.execute(
            "DELETE FROM mem_tree_summary_reembed_skipped WHERE model_signature = ?1",
            rusqlite::params![model_signature],
        )?;
        log::debug!(
            "[memory::chunk_store] clear_reembed_skipped_for_signature sig={model_signature} chunk_rows={chunk_deleted} summary_rows={summary_deleted}"
        );
        Ok(chunk_deleted + summary_deleted)
    })
}

/// Bounds attacker-controlled ids/signatures passed to reembed-skipped admin
/// helpers without affecting legitimate rows (typical ids are well under 512
/// chars). Rejects NUL bytes so SQLite bindings cannot be truncated.
const REEMBED_SKIP_KEY_MAX_LEN: usize = 2048;

pub(crate) fn validate_reembed_skip_key<'a>(label: &str, value: &'a str) -> Result<&'a str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{label} must be non-empty");
    }
    if trimmed.len() > REEMBED_SKIP_KEY_MAX_LEN {
        anyhow::bail!("{label} exceeds maximum length ({REEMBED_SKIP_KEY_MAX_LEN})");
    }
    if trimmed.as_bytes().contains(&0) {
        anyhow::bail!("{label} must not contain NUL bytes");
    }
    Ok(trimmed)
}

/// Transaction-scoped variant of [`set_chunk_embedding_for_signature`].
///
/// For callers that already hold a `Transaction` (e.g. the chunk-admission
/// handler, which commits the sidecar row in the SAME tx as the lifecycle
/// + score + job-enqueue writes — #1574 write-side cutover). Opening a fresh
/// connection there would break atomicity / deadlock on the busy DB.
pub(crate) fn set_chunk_embedding_for_signature_tx(
    tx: &rusqlite::Transaction<'_>,
    chunk_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    upsert_chunk_embedding_conn(tx, chunk_id, model_signature, embedding)
}

/// Fetch a chunk embedding for exactly one provider/model/dimension signature.
pub fn get_chunk_embedding_for_signature(
    config: &Config,
    chunk_id: &str,
    model_signature: &str,
) -> Result<Option<Vec<f32>>> {
    with_connection(config, |conn| {
        let row: Option<(Vec<u8>, i64)> = conn
            .query_row(
                "SELECT vector, dim
                   FROM mem_tree_chunk_embeddings
                  WHERE chunk_id = ?1 AND model_signature = ?2",
                rusqlite::params![chunk_id, model_signature],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((bytes, dim)) => embedding_from_blob(&bytes, dim, "chunk embedding"),
        }
    })
}

/// Fetch a chunk's embedding for the active model signature.
///
/// #1574 cutover: reads the per-model `mem_tree_chunk_embeddings` sidecar at
/// the active signature (via [`get_chunk_embedding_for_signature`]) instead of
/// the legacy `mem_tree_chunks.embedding` column. Returns `Ok(None)` if the
/// chunk has no vector under the active signature — e.g. during the §7
/// backfill window, where this degrades retrieval gracefully (the row is
/// simply absent from vector results, never cross-space compared).
pub fn get_chunk_embedding(config: &Config, chunk_id: &str) -> Result<Option<Vec<f32>>> {
    let signature = tree_active_signature(config);
    get_chunk_embedding_for_signature(config, chunk_id, &signature)
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn embedding_from_blob(bytes: &[u8], dim: i64, label: &str) -> Result<Option<Vec<f32>>> {
    if dim < 0 {
        anyhow::bail!("{label} has negative dimension {dim}");
    }
    if !bytes.len().is_multiple_of(4) {
        anyhow::bail!("{label} blob length {} not a multiple of 4", bytes.len());
    }
    let floats: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect();
    if floats.len() != dim as usize {
        anyhow::bail!(
            "{label} dimension mismatch: dim column says {dim}, blob contains {} floats",
            floats.len()
        );
    }
    Ok(Some(floats))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
