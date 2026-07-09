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
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::Arc;
use std::time::Duration;

use crate::openhuman::config::Config;
use crate::openhuman::memory::util::redact::{self, redact as redact_value};
use crate::openhuman::memory_store::chunks::types::{Chunk, SourceKind};
use crate::openhuman::memory_store::content::StagedChunk;
use crate::openhuman::tinycortex::memory_config_from;

const DB_DIR: &str = "memory_tree";
const DB_FILE: &str = "chunks.db";
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

/// `PRAGMA user_version` value once the global/topic-tree purge has run.
/// The global (time-axis) and topic (subject-axis) trees were removed; this
/// one-shot migration deletes their rows + on-disk summary folders. `< 2`
/// triggers the purge on next open; `>= 2` skips it.
const GLOBAL_TOPIC_PURGE_MIGRATION_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS mem_tree_chunks (
    id                     TEXT PRIMARY KEY,
    source_kind            TEXT NOT NULL,
    source_id              TEXT NOT NULL,
    path_scope             TEXT,
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

-- E2GraphRAG: undirected weighted entity co-occurrence graph. One row per
-- unordered entity pair that has been extracted together within the same
-- chunk; `weight` accumulates co-occurrence frequency. Canonical ordering
-- (`entity_a < entity_b`) keeps the pair unique without a second row, and
-- the `entity_b` index makes neighbour lookups symmetric (a row matches a
-- query entity whether it appears as `entity_a` or `entity_b`). Read at
-- query time by `memory_tree::graph` for bounded-hop shortest-path filtering
-- during deterministic (LLM-free) retrieval.
CREATE TABLE IF NOT EXISTS mem_tree_entity_edges (
    entity_a               TEXT NOT NULL,
    entity_b               TEXT NOT NULL,
    weight                 INTEGER NOT NULL DEFAULT 1,
    updated_ms             INTEGER NOT NULL,
    PRIMARY KEY (entity_a, entity_b)
);

CREATE INDEX IF NOT EXISTS idx_mem_tree_entity_edges_a
    ON mem_tree_entity_edges(entity_a);
CREATE INDEX IF NOT EXISTS idx_mem_tree_entity_edges_b
    ON mem_tree_entity_edges(entity_b);

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
    completed_at_ms        INTEGER,
    failure_reason         TEXT,
    failure_class          TEXT
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
    tinycortex::memory::chunks::upsert_chunks(&engine_config(config), chunks)
}

/// Upsert chunks using an existing transaction, preserving previously stored embeddings.
pub(crate) fn upsert_chunks_tx(tx: &Transaction<'_>, chunks: &[Chunk]) -> Result<usize> {
    if chunks.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT INTO mem_tree_chunks (
            id, source_kind, source_id, path_scope, source_ref, owner,
            timestamp_ms, time_range_start_ms, time_range_end_ms,
            tags_json, content, token_count, seq_in_source, created_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        ON CONFLICT(id) DO UPDATE SET
            source_kind = excluded.source_kind,
            source_id = excluded.source_id,
            path_scope = excluded.path_scope,
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
            id, source_kind, source_id, path_scope, source_ref, owner,
            timestamp_ms, time_range_start_ms, time_range_end_ms,
            tags_json, content, token_count, seq_in_source, created_at_ms,
            content_path, content_sha256
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        ON CONFLICT(id) DO UPDATE SET
            source_kind = excluded.source_kind,
            source_id = excluded.source_id,
            path_scope = excluded.path_scope,
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
            chunk.metadata.path_scope,
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

/// Repair the stored body-sha token for one chunk (#4689).
///
/// The chunk content file is content-addressed and atomically written, so it is
/// the source of truth for its body. When a read detects that the on-disk body
/// no longer hashes to the recorded `content_sha256` (e.g. an external editor
/// rewrote a synced file after ingest), the reader serves the full on-disk body
/// and calls this to re-point the stale token at the disk bytes so the next read
/// verifies cleanly instead of falling back to the ≤500-char preview.
pub fn update_chunk_content_sha256(
    config: &Config,
    chunk_id: &str,
    new_sha256: &str,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE mem_tree_chunks SET content_sha256 = ?1 WHERE id = ?2",
            params![new_sha256, chunk_id],
        )?;
        Ok(())
    })
}

/// Summary counterpart of [`update_chunk_content_sha256`] (#4689).
pub fn update_summary_content_sha256(
    config: &Config,
    summary_id: &str,
    new_sha256: &str,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "UPDATE mem_tree_summaries SET content_sha256 = ?1 WHERE id = ?2",
            params![new_sha256, summary_id],
        )?;
        Ok(())
    })
}

/// List the distinct `source_id`s of chunk rows for `source_kind` whose id
/// starts with `source_id_prefix` (#4689).
///
/// Used by folder/list-based resync to discover previously-ingested items that
/// have since vanished (e.g. a renamed or deleted file) so their stale rows +
/// on-disk bodies can be cleaned. The prefix is applied Rust-side (literal, not
/// a SQL `LIKE`) so ids containing `_`/`%` are matched verbatim — matching the
/// convention in [`delete_chunks_by_source_prefix`].
pub fn list_source_ids_with_prefix(
    config: &Config,
    source_kind: SourceKind,
    source_id_prefix: &str,
) -> Result<Vec<String>> {
    with_connection(config, |conn| {
        // Bounded range scan on the `idx_mem_tree_chunks_source (source_kind,
        // source_id)` index instead of scanning every source_id for the kind and
        // filtering in Rust: `source_id >= prefix AND source_id < upper`, where
        // `upper` is the prefix with its last byte incremented. With SQLite's
        // default BINARY collation this is exactly the literal byte-prefix set
        // (so `_`/`%` stay literal, matching `delete_chunks_by_source_prefix`).
        let out = match prefix_upper_bound(source_id_prefix) {
            Some(upper) => {
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT source_id FROM mem_tree_chunks \
                     WHERE source_kind = ?1 AND source_id >= ?2 AND source_id < ?3",
                )?;
                let rows = stmt.query_map(
                    params![source_kind.as_str(), source_id_prefix, upper],
                    |row| row.get::<_, String>(0),
                )?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            }
            None => {
                // Empty prefix (or all-0xFF): no finite upper bound. The lower
                // bound still uses the index; every row of the kind qualifies —
                // identical to the previous `starts_with("")` behaviour.
                let mut stmt = conn.prepare(
                    "SELECT DISTINCT source_id FROM mem_tree_chunks \
                     WHERE source_kind = ?1 AND source_id >= ?2",
                )?;
                let rows = stmt
                    .query_map(params![source_kind.as_str(), source_id_prefix], |row| {
                        row.get::<_, String>(0)
                    })?;
                rows.collect::<rusqlite::Result<Vec<_>>>()
            }
        }
        .context("Failed to list mem_tree chunk source_ids by prefix")?;
        Ok(out)
    })
}

/// Exclusive upper bound for a literal byte-prefix range scan: the least string
/// strictly greater than every string starting with `prefix`. `None` when no
/// finite bound exists (empty prefix, or every byte is `0xFF`), in which case
/// the caller applies only the lower bound.
fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut bytes = prefix.as_bytes().to_vec();
    while let Some(&last) = bytes.last() {
        if last < 0xFF {
            *bytes.last_mut().unwrap() = last + 1;
            // Our prefixes are ASCII (`mem_src:<id>:`), so incrementing the last
            // byte yields valid UTF-8. If a future caller passes a prefix that
            // ends mid-codepoint, fall back to the lower-bound-only path rather
            // than emit invalid UTF-8.
            return String::from_utf8(bytes).ok();
        }
        bytes.pop();
    }
    None
}

#[cfg(test)]
mod prefix_bound_tests {
    use super::prefix_upper_bound;

    #[test]
    fn upper_bound_increments_last_byte() {
        assert_eq!(
            prefix_upper_bound("mem_src:s1:").as_deref(),
            Some("mem_src:s1;")
        );
        assert_eq!(prefix_upper_bound("a").as_deref(), Some("b"));
    }

    #[test]
    fn upper_bound_none_for_empty_prefix() {
        // Empty prefix is the only reachable `None` for a valid UTF-8 `&str` (no
        // valid string ends in 0xFF); the all-0xFF `pop` loop is defensive.
        assert_eq!(prefix_upper_bound(""), None);
    }

    #[test]
    fn upper_bound_handles_multibyte_prefix() {
        // A prefix ending in a multibyte codepoint still yields a valid bound.
        assert_eq!(prefix_upper_bound("café").as_deref(), Some("cafê"));
    }

    #[test]
    fn range_excludes_sibling_prefix() {
        // The trailing delimiter in the prefix is what keeps `mem_src:s1:` from
        // matching `mem_src:s10:...`: `s10:` sorts below `s1:` (`'0' < ':'`), so
        // it falls below the lower bound and is excluded — same as `starts_with`.
        let prefix = "mem_src:s1:";
        let upper = prefix_upper_bound(prefix).unwrap();
        let sib = "mem_src:s10:x";
        assert!(!(sib >= prefix && *sib < *upper.as_str()));
        let child = "mem_src:s1:a";
        assert!(child >= prefix && *child < *upper.as_str());
    }
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
            chunk.metadata.path_scope,
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
/// Map the host `Config` to the engine `MemoryConfig` addressing the same
/// `<workspace_dir>/memory_tree/chunks.db` (only `workspace` is load-bearing for
/// these delegating DB reads). W3 store-op flip.
fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    memory_config_from(config, config.workspace_dir.clone())
}

pub fn get_chunk(config: &Config, id: &str) -> Result<Option<Chunk>> {
    tinycortex::memory::chunks::get_chunk(&engine_config(config), id)
}

/// Batched read of full chunk rows by id — delegates to the crate.
pub fn get_chunks_batch(config: &Config, chunk_ids: &[String]) -> Result<HashMap<String, Chunk>> {
    tinycortex::memory::chunks::get_chunks_batch(&engine_config(config), chunk_ids)
}

/// Query parameters for [`list_chunks`], re-exported from the crate (identical
/// fields incl. the `source_scope` allowlist + `exclude_dropped`).
pub use tinycortex::memory::chunks::ListChunksQuery;

/// List chunks matching the provided filters, ordered by `timestamp` DESC.
///
/// Delegates to the crate, which preserves the `source_scope` allowlist gate
/// (byte-identical `chunk_source_allowed_in` + `extract_mem_src_id`) — the
/// security-critical per-profile enforcement, pinned by
/// `store_tests::list_chunks_source_scope_filters_before_limit`.
pub fn list_chunks(config: &Config, query: &ListChunksQuery) -> Result<Vec<Chunk>> {
    tinycortex::memory::chunks::list_chunks(&engine_config(config), query)
}

/// Count total chunks in the store (useful for tests / diagnostics).
pub fn count_chunks(config: &Config) -> Result<u64> {
    tinycortex::memory::chunks::count_chunks(&engine_config(config))
}

/// #002 (FR-010 / US5): extraction coverage — the fraction of chunks that have
/// at least one indexed entity in `mem_tree_entity_index`, in `[0.0, 1.0]`.
///
/// Turns "wiki built / not built" into a quality signal: a value near 0 with a
/// non-zero chunk count means extraction is producing nothing (the model is
/// timing out / failing), even though chunks exist — the "empty-but-built
/// wiki" symptom. Joins the entity index against `mem_tree_chunks.id` so the
/// numerator is node-kind-agnostic (we only count entity rows whose `node_id`
/// is an actual chunk). Returns `0.0` when there are no chunks.
pub fn extraction_coverage(config: &Config) -> Result<f32> {
    tinycortex::memory::chunks::extraction_coverage(&engine_config(config))
}

/// Set the lifecycle status column for `chunk_id`. See `CHUNK_STATUS_*`.
pub fn set_chunk_lifecycle_status(config: &Config, chunk_id: &str, status: &str) -> Result<()> {
    tinycortex::memory::chunks::set_chunk_lifecycle_status(&engine_config(config), chunk_id, status)
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
    tinycortex::memory::chunks::get_chunk_lifecycle_status(&engine_config(config), chunk_id)
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
    tinycortex::memory::chunks::count_chunks_by_lifecycle_status(&engine_config(config), status)
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
    tinycortex::memory::chunks::is_source_ingested(&engine_config(config), source_kind, source_id)
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

/// `source_kind` value used in `mem_tree_ingested_sources` to record that a
/// raw archive file (relative path under `<content_root>/`, e.g.
/// `raw/github-com-org-repo/commits/<ts>_<sha>.md`) has been covered by a
/// tree summary. Distinct from the chunk-store [`SourceKind`] values so the
/// two gate namespaces can never collide.
pub const RAW_FILE_GATE_KIND: &str = "raw_file";

/// Record that the given raw archive files (relative paths under
/// `<content_root>/`) are covered by a tree summary. Idempotent
/// (`INSERT OR IGNORE`); returns the number of newly-recorded paths.
pub fn mark_raw_paths_ingested(config: &Config, rel_paths: &[String]) -> Result<u64> {
    tinycortex::memory::chunks::mark_raw_paths_ingested(&engine_config(config), rel_paths)
}

/// Filter `rel_paths` down to the ones NOT yet recorded as ingested raw
/// files. Order of the surviving paths is preserved.
pub fn filter_raw_paths_not_ingested(config: &Config, rel_paths: &[String]) -> Result<Vec<String>> {
    tinycortex::memory::chunks::filter_raw_paths_not_ingested(&engine_config(config), rel_paths)
}

/// Count raw-file gate rows whose path starts with `rel_prefix` (e.g.
/// `raw/github-com-org-repo/`). Diagnostic helper for reconcile reporting.
pub fn count_raw_paths_ingested_with_prefix(config: &Config, rel_prefix: &str) -> Result<u64> {
    tinycortex::memory::chunks::count_raw_paths_ingested_with_prefix(
        &engine_config(config),
        rel_prefix,
    )
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

        // A fully-orphaned source has zero chunks left, so its summary tree
        // now summarises deleted content — and its unsealed buffer holds
        // dangling chunk ids. Cascade-delete the tree (summaries + sidecars
        // + entity-index + buffer + tree row) so a `clear_memory` delete is
        // complete and stale summaries can't resurface in retrieval. Source
        // trees use the chunk `source_id` verbatim as their scope, so we
        // match on that. Same tx as the chunk delete → atomic.
        for source_id in &orphaned_deleted_sources {
            if let Some(tree) =
                crate::openhuman::memory_store::trees::store::get_tree_by_scope_conn(
                    &tx,
                    crate::openhuman::memory_store::trees::types::TreeKind::Source,
                    source_id,
                )?
            {
                let cascade = crate::openhuman::memory_store::trees::store::delete_tree_cascade_tx(
                    &tx, &tree.id,
                )?;
                // Defer the summary content-file removal to the same
                // post-commit sweep as the chunk files.
                content_paths.extend(cascade.content_paths);
                log::debug!(
                    "[memory::chunk_store] {op}: orphaned source_id_hash={} → deleted source tree tree_id={} summaries={}",
                    redact_value(source_id),
                    tree.id,
                    cascade.removed_summaries,
                );
            }
        }

        let deleted = chunks.len();
        tx.commit()?;
        Ok(deleted)
    })?;

    remove_chunk_content_files(config, &content_paths);
    Ok(deleted)
}

/// Finish off an orphaned **Source** (one with zero chunks remaining): clear its
/// ingest dedup gates and cascade-delete its source-scoped summary tree.
///
/// `delete_chunks_by_source` only cascades the tree for sources whose chunks it
/// deletes in the same call; a source whose chunks were already removed earlier
/// (e.g. by the per-chunk `delete_chunk` path) keeps a now-stale summary tree
/// that can still resurface in recall. This cleans up exactly that **legacy
/// partial-delete** state.
///
/// Specifically, when no chunks remain it:
/// - removes the ingest dedup gates for the source — both the bare `source_id`
///   AND any versioned `{source_id}@{version_ms}` gates (matched Rust-side with
///   exact/prefix comparison, never SQL `LIKE`/`GLOB`, to avoid metachar pitfalls);
/// - cascades the **source-scoped** tree (scope == `source_id`) if present.
///
/// Scoped-collection conservatism: a document ingested under a shared collection
/// `path_scope` (e.g. Notion `notion:{connection}`) lives in a tree scoped by that
/// `path_scope`, NOT by this `source_id`, so `get_tree_by_scope(Source,
/// source_id)` returns `None` and such shared trees are left intact — deleting one
/// document must never tear down a tree that summarises many documents.
///
/// Returns `true` when a source-scoped tree was removed (drives the RPC's
/// `deleted` flag). No-op-safe to call unconditionally after
/// `delete_chunks_by_source`.
pub fn delete_orphaned_source_tree(
    config: &Config,
    source_kind: SourceKind,
    source_id: &str,
) -> Result<bool> {
    use crate::openhuman::memory_store::trees::store as tree_store;
    use crate::openhuman::memory_store::trees::types::TreeKind;

    let mut content_paths: Vec<String> = Vec::new();
    let tree_cascaded = with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        let remaining: i64 = tx.query_row(
            "SELECT COUNT(*) FROM mem_tree_chunks WHERE source_kind = ?1 AND source_id = ?2",
            params![source_kind.as_str(), source_id],
            |r| r.get(0),
        )?;
        if remaining > 0 {
            // Source still has chunks — not orphaned; leave its live tree + gates.
            log::debug!(
                "[memory::chunk_store] delete_orphaned_source_tree: source_id_hash={} still has {remaining} chunk(s) — no-op",
                redact_value(source_id),
            );
            return Ok(false);
        }

        // Clear ALL ingest dedup gates for this source: the bare source_id and any
        // versioned `{source_id}@{version_ms}` gates. Filter in Rust (exact or
        // `source_id@` prefix) so `_`/`%`/glob chars in ids are treated literally.
        let versioned_prefix = format!("{source_id}@");
        let gate_ids: Vec<String> = {
            let mut stmt = tx.prepare(
                "SELECT source_id FROM mem_tree_ingested_sources WHERE source_kind = ?1",
            )?;
            let rows = stmt.query_map(params![source_kind.as_str()], |r| r.get::<_, String>(0))?;
            rows.filter_map(|row| match row {
                Ok(s) if s == source_id || s.starts_with(&versioned_prefix) => Some(Ok(s)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for gid in &gate_ids {
            tx.execute(
                "DELETE FROM mem_tree_ingested_sources WHERE source_kind = ?1 AND source_id = ?2",
                params![source_kind.as_str(), gid],
            )?;
        }

        // Cascade the source-scoped orphan tree if one exists. Shared
        // collection/path_scope trees are not keyed by this source_id (see fn
        // docs), so they are intentionally left untouched.
        let cascaded = if let Some(tree) =
            tree_store::get_tree_by_scope_conn(&tx, TreeKind::Source, source_id)?
        {
            let cascade = tree_store::delete_tree_cascade_tx(&tx, &tree.id)?;
            content_paths.extend(cascade.content_paths);
            log::debug!(
                    "[memory::chunk_store] delete_orphaned_source_tree: source_id_hash={} → removed stale tree_id={} summaries={} gates_cleared={}",
                    redact_value(source_id),
                    tree.id,
                    cascade.removed_summaries,
                    gate_ids.len(),
                );
            true
        } else {
            log::debug!(
                    "[memory::chunk_store] delete_orphaned_source_tree: source_id_hash={} has no source-scoped tree (gates_cleared={}); shared/collection trees left intact",
                    redact_value(source_id),
                    gate_ids.len(),
                );
            false
        };
        tx.commit()?;
        Ok(cascaded)
    })?;
    if tree_cascaded {
        remove_chunk_content_files(config, &content_paths);
    }
    Ok(tree_cascaded)
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

#[path = "connection.rs"]
mod connection;
pub(crate) use connection::recover_corrupt_db;
pub use connection::with_connection;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use connection::{
    clear_connection_cache, db_path_for, get_or_init_connection, invalidate_connection,
    is_io_open_error, schema_apply_count_for_path_for_tests, CB_THRESHOLD,
};
#[cfg(test)]
pub(crate) use connection::{is_transient_cold_start, try_cleanup_stale_files};

#[path = "migrations.rs"]
mod migrations;
use migrations::{migrate_legacy_embeddings_to_sidecar, purge_global_topic_trees};

#[path = "raw_refs.rs"]
mod raw_refs;
pub use raw_refs::{
    get_chunk_content_path, get_chunk_content_pointers, get_chunk_raw_refs,
    get_summary_content_pointers, list_chunk_raw_ref_paths_with_prefix,
    list_summaries_with_content_path, set_chunk_raw_refs, set_chunk_raw_refs_tx, RawRef,
};

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
