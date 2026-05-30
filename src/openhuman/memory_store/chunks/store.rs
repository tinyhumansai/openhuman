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
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use crate::openhuman::config::Config;
use crate::openhuman::memory::util::redact::{self, redact as redact_value};
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
// soft signal (see `memory_tree::jobs::worker`) so even if this is
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

-- MCP write-tool audit trail (#2536). This intentionally stores compact
-- identifying metadata instead of duplicating the memory document body.
CREATE TABLE IF NOT EXISTS mcp_writes (
    id                     INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp_ms           INTEGER NOT NULL,
    client_info            TEXT NOT NULL,
    tool_name              TEXT NOT NULL,
    args_summary           TEXT,
    resulting_chunk_id     TEXT,
    success                INTEGER NOT NULL,
    error_message          TEXT
);

CREATE INDEX IF NOT EXISTS idx_mcp_writes_timestamp
    ON mcp_writes(timestamp_ms DESC);
CREATE INDEX IF NOT EXISTS idx_mcp_writes_client
    ON mcp_writes(client_info);
CREATE INDEX IF NOT EXISTS idx_mcp_writes_tool
    ON mcp_writes(tool_name);
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

/// Defensive cap for batched `IN (?,?,…)` reads.
///
/// SQLite's compile-time limit on bound parameters in a single statement
/// (`SQLITE_MAX_VARIABLE_NUMBER`) has been **32 766** since 3.32 (2020),
/// so 500 leaves a ~65× safety margin. The current call-site
/// (`memory_tree::retrieval::fetch::fetch_leaves`) is capped at 20 ids,
/// so the chunked loop runs exactly once today. The window exists so
/// future call-sites passing larger id lists do not blow up against a
/// host with a lower compile-time SQLite cap (older builds, custom
/// embeddings, etc.).
///
/// Volume is **not** reduced: all input ids in → all matching rows out.
/// The loop only splits the SQL; the merged `HashMap` is byte-identical
/// to what one giant query would return.
const MAX_FETCH_BATCH: usize = 500;

/// Batched read of full chunk rows by id.
///
/// Contract mirror of looping [`get_chunk`] per id, but in
/// `O(ceil(n / MAX_FETCH_BATCH))` SQLite round-trips instead of `O(n)`.
/// The returned map contains only ids that exist in `mem_tree_chunks`;
/// missing ids are silently absent (same as `get_chunk` returning
/// `Ok(None)`). Callers that depend on input order must iterate their
/// own id slice and look each id up in the map.
///
/// Reuses [`row_to_chunk`] so decoding stays bit-identical to the
/// per-row helper — no risk of decoder drift.
pub fn get_chunks_batch(config: &Config, chunk_ids: &[String]) -> Result<HashMap<String, Chunk>> {
    if chunk_ids.is_empty() {
        return Ok(HashMap::new());
    }
    log::debug!(
        "[memory::chunk_store] get_chunks_batch: n={} windows={}",
        chunk_ids.len(),
        chunk_ids.len().div_ceil(MAX_FETCH_BATCH)
    );
    with_connection(config, |conn| {
        let mut out: HashMap<String, Chunk> = HashMap::with_capacity(chunk_ids.len());
        for window in chunk_ids.chunks(MAX_FETCH_BATCH) {
            // Build the placeholder list `?1, ?2, …, ?n` matching the
            // window length; rusqlite assigns positional binds 1..n in
            // the order the values are passed.
            let placeholders = (1..=window.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT id, source_kind, source_id, source_ref, owner,
                        timestamp_ms, time_range_start_ms, time_range_end_ms,
                        tags_json, content, token_count, seq_in_source, created_at_ms
                   FROM mem_tree_chunks WHERE id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).context("prepare get_chunks_batch")?;
            let params: Vec<&dyn rusqlite::ToSql> =
                window.iter().map(|id| id as &dyn rusqlite::ToSql).collect();
            let rows = stmt
                .query_map(params.as_slice(), row_to_chunk)
                .context("query get_chunks_batch")?;
            for row in rows {
                let chunk = row.context("decode get_chunks_batch row")?;
                out.insert(chunk.id.clone(), chunk);
            }
        }
        log::debug!(
            "[memory::chunk_store] get_chunks_batch: matched {}/{} ids",
            out.len(),
            chunk_ids.len()
        );
        Ok(out)
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

/// Delete all chunk rows for one exact `(source_kind, source_id)` and clear
/// dependent source-local indexes. Returns the number of chunk rows removed.
pub fn delete_chunks_by_source(
    config: &Config,
    source_kind: SourceKind,
    source_id: &str,
) -> Result<usize> {
    delete_chunks_by_source_filter(
        "delete_chunks_by_source",
        config,
        source_kind,
        |candidate, _owner| candidate == source_id,
        |candidate| candidate == source_id,
    )
}

/// Delete all chunk rows whose source id starts with `source_id_prefix`.
///
/// This is intentionally a Rust-side prefix filter rather than a SQL `LIKE`
/// expression so provider ids containing `_` / `%` are treated literally.
pub fn delete_chunks_by_source_prefix(
    config: &Config,
    source_kind: SourceKind,
    source_id_prefix: &str,
) -> Result<usize> {
    delete_chunks_by_source_filter(
        "delete_chunks_by_source_prefix",
        config,
        source_kind,
        |candidate, _owner| candidate.starts_with(source_id_prefix),
        |candidate| candidate.starts_with(source_id_prefix),
    )
}

/// Delete all chunk rows for one exact `(source_kind, owner)` while preserving
/// source ingest gates that still have chunks owned by another connection.
pub fn delete_chunks_by_owner(
    config: &Config,
    source_kind: SourceKind,
    owner: &str,
) -> Result<usize> {
    delete_chunks_by_source_filter(
        "delete_chunks_by_owner",
        config,
        source_kind,
        |_source_id, candidate_owner| candidate_owner == owner,
        |_source_id| false,
    )
}

fn delete_chunks_by_source_filter(
    op: &str,
    config: &Config,
    source_kind: SourceKind,
    matches_chunk: impl Fn(&str, &str) -> bool,
    matches_ingested_source: impl Fn(&str) -> bool,
) -> Result<usize> {
    let mut content_paths = Vec::new();
    let deleted = with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;

        let chunks = {
            let mut stmt = tx.prepare(
                "SELECT id, source_id, owner, content_path
                   FROM mem_tree_chunks
                  WHERE source_kind = ?1",
            )?;
            let rows = stmt.query_map(params![source_kind.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            })?;
            rows.filter_map(|row| match row {
                Ok((id, source_id, owner, content_path)) if matches_chunk(&source_id, &owner) => {
                    Some(Ok((id, source_id, content_path)))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect memory_tree chunks by source")?
        };

        let deleted_source_ids: HashSet<String> = chunks
            .iter()
            .map(|(_, source_id, _)| source_id.clone())
            .collect();

        for (chunk_id, _source_id, content_path) in &chunks {
            tx.execute(
                "DELETE FROM mem_tree_score WHERE chunk_id = ?1",
                params![chunk_id],
            )?;
            tx.execute(
                "DELETE FROM mem_tree_entity_index WHERE node_id = ?1",
                params![chunk_id],
            )?;
            tx.execute(
                "DELETE FROM mem_tree_chunk_embeddings WHERE chunk_id = ?1",
                params![chunk_id],
            )?;
            tx.execute(
                "DELETE FROM mem_tree_chunk_reembed_skipped WHERE chunk_id = ?1",
                params![chunk_id],
            )?;
            tx.execute(
                "DELETE FROM mem_tree_chunks WHERE id = ?1",
                params![chunk_id],
            )?;
            if let Some(path) = content_path.as_ref().filter(|path| !path.is_empty()) {
                content_paths.push(path.clone());
            }
        }

        let mut orphaned_deleted_sources = HashSet::new();
        for source_id in &deleted_source_ids {
            let remaining: i64 = tx.query_row(
                "SELECT COUNT(*)
                   FROM mem_tree_chunks
                  WHERE source_kind = ?1 AND source_id = ?2",
                params![source_kind.as_str(), source_id],
                |row| row.get(0),
            )?;
            if remaining == 0 {
                log::debug!(
                    "[memory::chunk_store] {op}: source_id_hash={} orphaned; removing ingest gate",
                    redact_value(source_id),
                );
                orphaned_deleted_sources.insert(source_id.clone());
            } else {
                log::debug!(
                    "[memory::chunk_store] {op}: source_id_hash={} remaining_chunks={remaining}; preserving ingest gate",
                    redact_value(source_id),
                );
            }
        }

        let ingested_sources = {
            let mut stmt = tx.prepare(
                "SELECT source_id
                   FROM mem_tree_ingested_sources
                  WHERE source_kind = ?1",
            )?;
            let rows =
                stmt.query_map(params![source_kind.as_str()], |row| row.get::<_, String>(0))?;
            rows.filter_map(|row| match row {
                Ok(source_id)
                    if matches_ingested_source(&source_id)
                        || orphaned_deleted_sources.contains(&source_id) =>
                {
                    Some(Ok(source_id))
                }
                Ok(_) => None,
                Err(error) => Some(Err(error)),
            })
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect memory_tree ingested sources")?
        };

        for source_id in &ingested_sources {
            tx.execute(
                "DELETE FROM mem_tree_ingested_sources
                  WHERE source_kind = ?1 AND source_id = ?2",
                params![source_kind.as_str(), source_id],
            )?;
        }

        let deleted = chunks.len();
        tx.commit()?;
        Ok(deleted)
    })?;

    remove_chunk_content_files(config, &content_paths);
    Ok(deleted)
}

fn remove_chunk_content_files(config: &Config, content_paths: &[String]) {
    use std::path::{Component, Path};

    let root = config.memory_tree_content_root();
    let canonical_root = match std::fs::canonicalize(&root) {
        Ok(path) => path,
        Err(error) => {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "[memory_tree::store] failed to resolve content root {}: {error}",
                    root.display(),
                );
            }
            return;
        }
    };

    for rel in content_paths {
        let rel_path = Path::new(rel);
        let has_escape_component = rel_path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
        if has_escape_component {
            log::warn!(
                "[memory_tree::store] refusing to remove chunk file with unsafe content_path path_hash={}",
                redact::redact(rel),
            );
            continue;
        }

        let path = root.join(rel_path);
        let resolved_path = match std::fs::canonicalize(&path) {
            Ok(path) => path,
            Err(error) => {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "[memory_tree::store] failed to resolve chunk file path_hash={}: {error}",
                        redact::redact(rel),
                    );
                }
                continue;
            }
        };
        if !resolved_path.starts_with(&canonical_root) {
            log::warn!(
                "[memory_tree::store] refusing to remove chunk file outside content root path_hash={}",
                redact::redact(rel),
            );
            continue;
        }

        if let Err(error) = std::fs::remove_file(&path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                log::warn!(
                    "[memory_tree::store] failed to remove chunk file path_hash={}: {error}",
                    redact::redact(rel),
                );
            }
        }
    }
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

#[path = "connection.rs"]
mod connection;
pub use connection::with_connection;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use connection::{
    clear_connection_cache, db_path_for, get_or_init_connection, invalidate_connection,
    is_io_open_error, schema_apply_count_for_path_for_tests, CB_THRESHOLD,
};
#[cfg(test)]
pub(crate) use connection::{is_transient_cold_start, try_cleanup_stale_files};

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
        let backfill_job = crate::openhuman::memory_queue::types::NewJob::reembed_backfill(
            &crate::openhuman::memory_queue::types::ReembedBackfillPayload {
                signature: sig.clone(),
            },
        )?;
        crate::openhuman::memory_queue::enqueue_tx(&tx, &backfill_job)?;
    }

    tx.commit()?;
    conn.pragma_update(None, "user_version", TREE_EMBEDDING_MIGRATION_VERSION)
        .context("set PRAGMA user_version after #1574 migration")?;
    if has_uncovered {
        crate::openhuman::memory_queue::set_backfill_in_progress(true);
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

#[path = "embeddings.rs"]
mod embeddings;
pub use embeddings::{
    clear_chunk_reembed_skipped, clear_reembed_skipped_for_signature, get_chunk_embedding,
    get_chunk_embedding_for_signature, get_chunk_embeddings_batch,
    get_chunk_embeddings_for_signature_batch, mark_chunk_reembed_skipped, set_chunk_embedding,
    set_chunk_embedding_for_signature,
};
#[cfg(test)]
pub(crate) use embeddings::{embedding_to_blob, REEMBED_SKIP_KEY_MAX_LEN};
pub(crate) use embeddings::{
    has_uncovered_reembed_work, set_chunk_embedding_for_signature_tx, tree_active_signature,
    validate_reembed_skip_key,
};
// ── Phase 2: embedding column accessors ─────────────────────────────────

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
