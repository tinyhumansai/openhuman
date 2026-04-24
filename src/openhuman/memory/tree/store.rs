//! SQLite-backed persistence for ingested chunks (Phase 1 / issue #707).
//!
//! The store lives at `<workspace>/memory_tree/chunks.db`. Schema is applied
//! lazily on first access via `with_connection`, so the DB is created on
//! demand without an explicit migration step.
//!
//! Upsert semantics: writes are idempotent on `chunk.id` so re-ingesting the
//! same raw source yields no duplicates.

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use std::time::Duration;

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::types::{Chunk, Metadata, SourceKind, SourceRef};

const DB_DIR: &str = "memory_tree";
const DB_FILE: &str = "chunks.db";
const DEFAULT_LIST_LIMIT: usize = 100;
const MAX_LIST_LIMIT: usize = 10_000;
const SQLITE_BUSY_TIMEOUT: Duration = Duration::from_secs(5);

const SCHEMA: &str = "
PRAGMA foreign_keys = ON;

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
CREATE TABLE IF NOT EXISTS mem_tree_entity_index (
    entity_id              TEXT NOT NULL,
    node_id                TEXT NOT NULL,
    node_kind              TEXT NOT NULL,
    entity_kind            TEXT NOT NULL,
    surface                TEXT NOT NULL,
    score                  REAL NOT NULL,
    timestamp_ms           INTEGER NOT NULL,
    tree_id                TEXT,
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
        "[memory_tree::store] upsert_chunks: n={} first_id={}",
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

/// Open the memory_tree SQLite DB and run a closure against it.
///
/// Visible to sibling modules (e.g. `score::store`) so Phase 2 can reuse
/// the same connection setup / schema initialisation without duplication.
pub(crate) fn with_connection<T>(
    config: &Config,
    f: impl FnOnce(&Connection) -> Result<T>,
) -> Result<T> {
    let dir = config.workspace_dir.join(DB_DIR);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("Failed to create memory_tree dir: {}", dir.display()))?;
    let db_path = dir.join(DB_FILE);
    let conn = Connection::open(&db_path)
        .with_context(|| format!("Failed to open memory_tree DB: {}", db_path.display()))?;
    conn.busy_timeout(SQLITE_BUSY_TIMEOUT)
        .context("Failed to configure memory_tree busy timeout")?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")
        .context("Failed to enable memory_tree WAL mode")?;
    conn.execute_batch(SCHEMA)
        .context("Failed to initialize memory_tree schema")?;
    // Phase 2 migrations — additive, idempotent.
    add_column_if_missing(&conn, "mem_tree_chunks", "embedding", "BLOB")?;
    // Phase 2 LLM-NER follow-up: per-chunk LLM importance signal +
    // human-readable reason. Both nullable; absence is treated as
    // "no LLM signal available" by readers.
    add_column_if_missing(&conn, "mem_tree_score", "llm_importance", "REAL")?;
    add_column_if_missing(&conn, "mem_tree_score", "llm_importance_reason", "TEXT")?;
    // Phase 3a (#709): parent-summary backlink on leaves. Populated when
    // the L0 buffer seals into an L1 summary so traversal can walk
    // leaf → parent without scanning `mem_tree_summaries.child_ids_json`.
    add_column_if_missing(&conn, "mem_tree_chunks", "parent_summary_id", "TEXT")?;
    // Phase 4 (#710): sealed-summary embeddings for semantic rerank.
    // Blob layout matches `mem_tree_chunks.embedding` — see
    // `score::embed::{pack_embedding, unpack_embedding}`. Nullable so
    // legacy summaries from Phases 1-3 read back as None; retrieval
    // tolerates NULL by dropping the row to the bottom of a rerank.
    add_column_if_missing(&conn, "mem_tree_summaries", "embedding", "BLOB")?;
    f(&conn)
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
            log::debug!("[memory_tree::store] migration: added column {table}.{name} ({sql_type})");
            Ok(())
        }
        Err(err) if err.to_string().contains("duplicate column name") => Ok(()),
        Err(err) => Err(err).with_context(|| format!("Failed to add column {table}.{name}")),
    }
}

// ── Phase 2: embedding column accessors ─────────────────────────────────

/// Store a chunk's embedding as a packed little-endian `f32` blob.
///
/// Length is `embedding.len() * 4` bytes. The caller is responsible for
/// ensuring all embeddings in a given deployment share the same dimension.
pub fn set_chunk_embedding(config: &Config, chunk_id: &str, embedding: &[f32]) -> Result<()> {
    let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
    with_connection(config, |conn| {
        let changed = conn.execute(
            "UPDATE mem_tree_chunks SET embedding = ?1 WHERE id = ?2",
            rusqlite::params![bytes, chunk_id],
        )?;
        if changed == 0 {
            log::warn!("[memory_tree::store] set_chunk_embedding: no row for chunk_id={chunk_id}");
        }
        Ok(())
    })
}

/// Fetch a chunk's embedding, decoding the stored little-endian `f32` blob.
///
/// Returns `Ok(None)` if the chunk doesn't exist or has no embedding stored.
pub fn get_chunk_embedding(config: &Config, chunk_id: &str) -> Result<Option<Vec<f32>>> {
    with_connection(config, |conn| {
        let blob: Option<Option<Vec<u8>>> = conn
            .query_row(
                "SELECT embedding FROM mem_tree_chunks WHERE id = ?1",
                rusqlite::params![chunk_id],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?;
        match blob.flatten() {
            None => Ok(None),
            Some(bytes) => {
                if !bytes.len().is_multiple_of(4) {
                    anyhow::bail!("embedding blob length {} not a multiple of 4", bytes.len());
                }
                let floats: Vec<f32> = bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                Ok(Some(floats))
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::types::chunk_id;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().expect("tempdir");
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn sample_chunk(source_id: &str, seq: u32, ts_ms: i64) -> Chunk {
        let ts = Utc.timestamp_millis_opt(ts_ms).unwrap();
        Chunk {
            id: chunk_id(SourceKind::Chat, source_id, seq),
            content: format!("content {source_id} {seq}"),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: source_id.to_string(),
                owner: "alice@example.com".to_string(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["eng".into()],
                source_ref: Some(SourceRef::new(format!("slack://{source_id}/{seq}"))),
            },
            token_count: 12,
            seq_in_source: seq,
            created_at: ts,
        }
    }

    #[test]
    fn upsert_then_get() {
        let (_tmp, cfg) = test_config();
        let c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
        assert_eq!(upsert_chunks(&cfg, &[c.clone()]).unwrap(), 1);
        let got = get_chunk(&cfg, &c.id).unwrap().expect("chunk stored");
        assert_eq!(got, c);
    }

    #[test]
    fn upsert_is_idempotent() {
        let (_tmp, cfg) = test_config();
        let c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        assert_eq!(count_chunks(&cfg).unwrap(), 1);
    }

    #[test]
    fn reingest_preserves_existing_embedding() {
        let (_tmp, cfg) = test_config();
        let mut c = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
        upsert_chunks(&cfg, &[c.clone()]).unwrap();
        set_chunk_embedding(&cfg, &c.id, &[0.1, 0.2, 0.3]).unwrap();

        c.content = "updated content".into();
        c.token_count = 99;
        upsert_chunks(&cfg, &[c.clone()]).unwrap();

        let embedding = get_chunk_embedding(&cfg, &c.id).unwrap().unwrap();
        assert_eq!(embedding, vec![0.1, 0.2, 0.3]);
        let got = get_chunk(&cfg, &c.id).unwrap().unwrap();
        assert_eq!(got.content, "updated content");
        assert_eq!(got.token_count, 99);
    }

    #[test]
    fn list_filters_by_source_kind() {
        let (_tmp, cfg) = test_config();
        let c1 = sample_chunk("slack:#eng", 0, 1_700_000_000_000);
        let mut c2 = sample_chunk("gmail:t1", 0, 1_700_000_001_000);
        c2.metadata.source_kind = SourceKind::Email;
        upsert_chunks(&cfg, &[c1.clone(), c2.clone()]).unwrap();
        let q = ListChunksQuery {
            source_kind: Some(SourceKind::Email),
            ..Default::default()
        };
        let rows = list_chunks(&cfg, &q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].metadata.source_kind, SourceKind::Email);
    }

    #[test]
    fn list_filters_by_time_range() {
        let (_tmp, cfg) = test_config();
        let a = sample_chunk("s", 0, 1_700_000_000_000);
        let b = sample_chunk("s", 1, 1_700_000_010_000);
        let c = sample_chunk("s", 2, 1_700_000_020_000);
        upsert_chunks(&cfg, &[a.clone(), b.clone(), c.clone()]).unwrap();
        let q = ListChunksQuery {
            since_ms: Some(1_700_000_005_000),
            until_ms: Some(1_700_000_015_000),
            ..Default::default()
        };
        let rows = list_chunks(&cfg, &q).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, b.id);
    }

    #[test]
    fn list_orders_by_timestamp_desc() {
        let (_tmp, cfg) = test_config();
        let a = sample_chunk("s", 0, 1_700_000_000_000);
        let b = sample_chunk("s", 1, 1_700_000_010_000);
        upsert_chunks(&cfg, &[a.clone(), b.clone()]).unwrap();
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, b.id); // newest first
        assert_eq!(rows[1].id, a.id);
    }

    #[test]
    fn list_orders_equal_timestamps_by_sequence() {
        let (_tmp, cfg) = test_config();
        let a = sample_chunk("s", 0, 1_700_000_000_000);
        let b = sample_chunk("s", 1, 1_700_000_000_000);
        upsert_chunks(&cfg, &[b.clone(), a.clone()]).unwrap();
        let rows = list_chunks(&cfg, &ListChunksQuery::default()).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].seq_in_source, 0);
        assert_eq!(rows[1].seq_in_source, 1);
    }

    #[test]
    fn list_limit_is_clamped_to_sane_range() {
        let (_tmp, cfg) = test_config();
        let chunks = (0..3)
            .map(|idx| sample_chunk("s", idx, 1_700_000_000_000 + i64::from(idx)))
            .collect::<Vec<_>>();
        upsert_chunks(&cfg, &chunks).unwrap();

        let zero_limit = list_chunks(
            &cfg,
            &ListChunksQuery {
                limit: Some(0),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(zero_limit.len(), 1);

        let huge_limit = list_chunks(
            &cfg,
            &ListChunksQuery {
                limit: Some(usize::MAX),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(huge_limit.len(), 3);
    }

    #[test]
    fn missing_chunk_returns_none() {
        let (_tmp, cfg) = test_config();
        assert!(get_chunk(&cfg, "nonexistent").unwrap().is_none());
    }

    #[test]
    fn empty_batch_is_noop() {
        let (_tmp, cfg) = test_config();
        assert_eq!(upsert_chunks(&cfg, &[]).unwrap(), 0);
        assert_eq!(count_chunks(&cfg).unwrap(), 0);
    }
}
