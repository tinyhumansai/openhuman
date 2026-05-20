//! SQLite-backed persistence for Phase 3a summary trees (#709).
//!
//! Three tables (schema lives in the sibling `tree::store::SCHEMA`):
//! - `mem_tree_trees`      — one row per tree (kind, scope, root, max_level)
//! - `mem_tree_summaries`  — one row per sealed summary node (immutable)
//! - `mem_tree_buffers`    — one row per unsealed frontier `(tree_id, level)`
//!
//! All timestamps are stored as milliseconds since the Unix epoch so we
//! share the epoch convention with `mem_tree_chunks`. Writes are serialised
//! through the sibling `tree::store::with_connection` so we inherit its
//! busy-timeout, WAL, and schema-init behaviour.
//!
//! Phase 4 (#710) adds a nullable `embedding` blob on
//! `mem_tree_summaries` — packed little-endian `f32` vectors via
//! [`crate::openhuman::memory::tree::score::embed::pack_embedding`]. New
//! writes populate it via [`insert_summary_tx`]; reads decode it when
//! present.

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::content_store::StagedSummary;
use crate::openhuman::memory::tree::score::embed::{decode_optional_blob, pack_checked};
use crate::openhuman::memory::tree::store::with_connection;
use crate::openhuman::memory::tree::tree_source::types::{
    Buffer, SummaryNode, Tree, TreeKind, TreeStatus,
};

fn ms_to_utc(ms: i64) -> rusqlite::Result<DateTime<Utc>> {
    Utc.timestamp_millis_opt(ms).single().ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            format!("invalid timestamp ms {ms}").into(),
        )
    })
}

// ── Tree rows ───────────────────────────────────────────────────────────

/// Insert a new tree row. Fails if `(kind, scope)` already exists; callers
/// that want "get or create" semantics should go through the `registry`.
pub fn insert_tree(config: &Config, tree: &Tree) -> Result<()> {
    with_connection(config, |conn| insert_tree_conn(conn, tree))
}

pub(crate) fn insert_tree_conn(conn: &Connection, tree: &Tree) -> Result<()> {
    conn.execute(
        "INSERT INTO mem_tree_trees (
            id, kind, scope, root_id, max_level, status,
            created_at_ms, last_sealed_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            tree.id,
            tree.kind.as_str(),
            tree.scope,
            tree.root_id,
            tree.max_level,
            tree.status.as_str(),
            tree.created_at.timestamp_millis(),
            tree.last_sealed_at.map(|t| t.timestamp_millis()),
        ],
    )
    .with_context(|| format!("Failed to insert tree id={}", tree.id))?;
    Ok(())
}

/// Fetch a tree by `(kind, scope)`. Returns `None` if no such tree exists.
pub fn get_tree_by_scope(config: &Config, kind: TreeKind, scope: &str) -> Result<Option<Tree>> {
    with_connection(config, |conn| get_tree_by_scope_conn(conn, kind, scope))
}

pub(crate) fn get_tree_by_scope_conn(
    conn: &Connection,
    kind: TreeKind,
    scope: &str,
) -> Result<Option<Tree>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, scope, root_id, max_level, status,
                created_at_ms, last_sealed_at_ms
           FROM mem_tree_trees WHERE kind = ?1 AND scope = ?2",
    )?;
    let row = stmt
        .query_row(params![kind.as_str(), scope], row_to_tree)
        .optional()
        .context("Failed to query tree by scope")?;
    Ok(row)
}

/// Fetch a tree by primary key id.
pub fn get_tree(config: &Config, id: &str) -> Result<Option<Tree>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, kind, scope, root_id, max_level, status,
                    created_at_ms, last_sealed_at_ms
               FROM mem_tree_trees WHERE id = ?1",
        )?;
        let row = stmt
            .query_row(params![id], row_to_tree)
            .optional()
            .context("Failed to query tree by id")?;
        Ok(row)
    })
}

/// List every tree of a given kind. Used by the global digest to enumerate
/// source trees, and by diagnostics. Rows come back ordered by `created_at_ms`
/// ASC so callers see a stable iteration order.
pub fn list_trees_by_kind(config: &Config, kind: TreeKind) -> Result<Vec<Tree>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, kind, scope, root_id, max_level, status,
                    created_at_ms, last_sealed_at_ms
               FROM mem_tree_trees
              WHERE kind = ?1
              ORDER BY created_at_ms ASC",
        )?;
        let rows = stmt
            .query_map(params![kind.as_str()], row_to_tree)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect trees by kind")?;
        Ok(rows)
    })
}

pub(crate) fn update_tree_after_seal_tx(
    tx: &Transaction<'_>,
    tree_id: &str,
    root_id: &str,
    max_level: u32,
    sealed_at: DateTime<Utc>,
) -> Result<()> {
    tx.execute(
        "UPDATE mem_tree_trees
            SET root_id = ?1,
                max_level = ?2,
                last_sealed_at_ms = ?3
          WHERE id = ?4",
        params![root_id, max_level, sealed_at.timestamp_millis(), tree_id,],
    )
    .with_context(|| format!("Failed to update tree {tree_id} after seal"))?;
    Ok(())
}

fn row_to_tree(row: &rusqlite::Row<'_>) -> rusqlite::Result<Tree> {
    let id: String = row.get(0)?;
    let kind_s: String = row.get(1)?;
    let scope: String = row.get(2)?;
    let root_id: Option<String> = row.get(3)?;
    let max_level: i64 = row.get(4)?;
    let status_s: String = row.get(5)?;
    let created_ms: i64 = row.get(6)?;
    let last_sealed_ms: Option<i64> = row.get(7)?;

    let kind = TreeKind::parse(&kind_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(1, rusqlite::types::Type::Text, e.into())
    })?;
    let status = TreeStatus::parse(&status_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, e.into())
    })?;
    Ok(Tree {
        id,
        kind,
        scope,
        root_id,
        max_level: max_level.max(0) as u32,
        status,
        created_at: ms_to_utc(created_ms)?,
        last_sealed_at: last_sealed_ms.map(ms_to_utc).transpose()?,
    })
}

// ── Summary nodes ───────────────────────────────────────────────────────

/// Insert a sealed summary. Immutable — the caller must generate a fresh
/// id per seal. Idempotent on the primary key so retries of the same seal
/// transaction don't double-insert.
///
/// Phase 4 (#710): if `node.embedding` is `Some`, the packed vector is
/// written to the `embedding` blob column; `None` writes NULL so legacy
/// rows from Phases 1-3 (no embed) read back identically.
///
/// Phase MD-content: if `staged` is `Some`, writes `content_path` and
/// `content_sha256` and truncates `content` to a ≤500-char preview. Callers
/// that have not yet staged the file pass `None`, in which case the full
/// `node.content` is stored (legacy behaviour).
pub(crate) fn insert_summary_tx(
    tx: &Transaction<'_>,
    node: &SummaryNode,
    staged: Option<&StagedSummary>,
    model_signature: &str,
) -> Result<()> {
    // #1574 write-side cutover: keep the dimension guard (fail the seal fast
    // on a misconfigured embedder) but DO NOT write the legacy
    // `mem_tree_summaries.embedding` column — the vector is persisted to the
    // per-model sidecar below, in THIS tx so it commits atomically with the
    // summary row. The legacy column is left NULL for the §7 migration.
    if let Some(v) = node.embedding.as_deref() {
        pack_checked(v)
            .with_context(|| format!("validate embedding dims for summary id={}", node.id))?;
    }
    let embedding_blob: Option<Vec<u8>> = None;

    // Phase MD-content: when a staged file exists, truncate `content` to a
    // ≤500-char plain-text preview (char boundary safe via chars().take(500)).
    let (content_preview, content_path, content_sha256) = match staged {
        Some(s) => {
            let preview: String = node.content.chars().take(500).collect();
            (
                preview,
                Some(s.content_path.clone()),
                Some(s.content_sha256.clone()),
            )
        }
        None => (node.content.clone(), None, None),
    };

    tx.execute(
        "INSERT OR IGNORE INTO mem_tree_summaries (
            id, tree_id, tree_kind, level, parent_id,
            child_ids_json, content, token_count,
            entities_json, topics_json,
            time_range_start_ms, time_range_end_ms,
            score, sealed_at_ms, deleted, embedding,
            content_path, content_sha256
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
        params![
            node.id,
            node.tree_id,
            node.tree_kind.as_str(),
            node.level,
            node.parent_id,
            serde_json::to_string(&node.child_ids)?,
            content_preview,
            node.token_count,
            serde_json::to_string(&node.entities)?,
            serde_json::to_string(&node.topics)?,
            node.time_range_start.timestamp_millis(),
            node.time_range_end.timestamp_millis(),
            node.score,
            node.sealed_at.timestamp_millis(),
            node.deleted as i64,
            embedding_blob,
            content_path,
            content_sha256,
        ],
    )
    .with_context(|| format!("Failed to insert summary id={}", node.id))?;

    // #1574: persist the embedding to the per-model sidecar at the active
    // signature, in the SAME tx as the summary row insert above.
    if let Some(v) = node.embedding.as_deref() {
        upsert_summary_embedding_conn(tx, &node.id, model_signature, v)?;
    }
    Ok(())
}

/// Set (or overwrite) the embedding for an existing summary row.
///
/// #1574 cutover: writes the per-model `mem_tree_summary_embeddings` sidecar
/// at the active signature (via [`set_summary_embedding_for_signature`])
/// instead of the legacy `mem_tree_summaries.embedding` column. The signature
/// is resolved internally from `config` via the shared
/// [`crate::openhuman::memory::tree::store::tree_active_signature`] — same
/// resolution as the chunk path. Returns `1` on success (one sidecar row
/// written/updated); the legacy "0 if id unknown" count no longer applies
/// since the sidecar upsert does not join the parent summary row.
pub fn set_summary_embedding(
    config: &Config,
    summary_id: &str,
    embedding: &[f32],
) -> Result<usize> {
    let signature = crate::openhuman::memory::tree::store::tree_active_signature(config);
    log::debug!(
        "[tree_source::store] set_summary_embedding: summary_id={summary_id} sig={signature} dims={}",
        embedding.len()
    );
    set_summary_embedding_for_signature(config, summary_id, &signature, embedding)?;
    Ok(1)
}

/// Fetch a summary's embedding for the active model signature.
///
/// #1574 cutover: reads the per-model `mem_tree_summary_embeddings` sidecar at
/// the active signature (via [`get_summary_embedding_for_signature`]) instead
/// of the legacy `mem_tree_summaries.embedding` column. `Ok(None)` when no
/// vector exists under the active signature — graceful absence during the §7
/// backfill window, never a cross-space read.
pub fn get_summary_embedding(config: &Config, summary_id: &str) -> Result<Option<Vec<f32>>> {
    let signature = crate::openhuman::memory::tree::store::tree_active_signature(config);
    get_summary_embedding_for_signature(config, summary_id, &signature)
}

/// Core upsert into `mem_tree_summary_embeddings` over an arbitrary
/// `&Connection`. Shared by the standalone
/// ([`set_summary_embedding_for_signature`]) and in-transaction
/// ([`set_summary_embedding_for_signature_tx`]) write paths so the SQL exists
/// exactly once. `rusqlite::Transaction` derefs to `Connection`, so the seal
/// path passes `&tx` and the sidecar row commits atomically with the summary
/// row insert (#1574 write-side cutover).
fn upsert_summary_embedding_conn(
    conn: &rusqlite::Connection,
    summary_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    let blob = pack_embedding_blob(embedding);
    let dim = i64::try_from(embedding.len()).context("embedding dimension does not fit i64")?;
    let created_at = Utc::now().timestamp_millis() as f64 / 1000.0;
    conn.execute(
        "INSERT INTO mem_tree_summary_embeddings
             (summary_id, model_signature, vector, dim, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(summary_id, model_signature) DO UPDATE SET
                vector = excluded.vector,
                dim = excluded.dim,
                created_at = excluded.created_at",
        params![summary_id, model_signature, blob, dim, created_at],
    )?;
    Ok(())
}

/// Store a summary embedding for a specific provider/model/dimension signature.
///
/// Per-model table write path for #1574. The legacy
/// `mem_tree_summaries.embedding` column is intentionally left untouched by
/// this helper (read by the §7 migration; dropped only in a later release).
pub fn set_summary_embedding_for_signature(
    config: &Config,
    summary_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    with_connection(config, |conn| {
        upsert_summary_embedding_conn(conn, summary_id, model_signature, embedding)
    })
}

/// Transaction-scoped variant of [`set_summary_embedding_for_signature`], for
/// the seal path which inserts the summary row and its embedding in one tx
/// (#1574 write-side cutover). Opening a fresh connection there would break
/// atomicity / deadlock on the busy DB.
pub(crate) fn set_summary_embedding_for_signature_tx(
    tx: &rusqlite::Transaction<'_>,
    summary_id: &str,
    model_signature: &str,
    embedding: &[f32],
) -> Result<()> {
    upsert_summary_embedding_conn(tx, summary_id, model_signature, embedding)
}

/// Fetch a summary embedding for exactly one provider/model/dimension signature.
pub fn get_summary_embedding_for_signature(
    config: &Config,
    summary_id: &str,
    model_signature: &str,
) -> Result<Option<Vec<f32>>> {
    with_connection(config, |conn| {
        let row: Option<(Option<Vec<u8>>, i64)> = conn
            .query_row(
                "SELECT vector, dim
                   FROM mem_tree_summary_embeddings
                  WHERE summary_id = ?1 AND model_signature = ?2",
                params![summary_id, model_signature],
                |r| Ok((Some(r.get(0)?), r.get(1)?)),
            )
            .optional()?;
        match row {
            None => Ok(None),
            Some((blob, dim)) => {
                let decoded =
                    decode_signature_blob(blob, dim, &format!("summary_id={summary_id}"))?;
                if decoded.as_ref().is_some_and(|v| v.len() != dim as usize) {
                    anyhow::bail!(
                        "summary embedding dimension mismatch: dim column says {dim}, blob contains {} floats",
                        decoded.as_ref().map_or(0, Vec::len)
                    );
                }
                Ok(decoded)
            }
        }
    })
}

/// Fetch one summary by id. Soft-deleted rows are returned with
/// `deleted = true` so callers can decide filtering policy.
pub fn get_summary(config: &Config, id: &str) -> Result<Option<SummaryNode>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, tree_id, tree_kind, level, parent_id,
                    child_ids_json, content, token_count,
                    entities_json, topics_json,
                    time_range_start_ms, time_range_end_ms,
                    score, sealed_at_ms, deleted, embedding
               FROM mem_tree_summaries WHERE id = ?1",
        )?;
        let row = stmt
            .query_row(params![id], row_to_summary)
            .optional()
            .context("Failed to query summary by id")?;
        Ok(row)
    })
}

/// List sealed summaries for a tree at a given level, ordered by
/// `sealed_at` ascending. Skips tombstoned rows.
pub fn list_summaries_at_level(
    config: &Config,
    tree_id: &str,
    level: u32,
) -> Result<Vec<SummaryNode>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT id, tree_id, tree_kind, level, parent_id,
                    child_ids_json, content, token_count,
                    entities_json, topics_json,
                    time_range_start_ms, time_range_end_ms,
                    score, sealed_at_ms, deleted, embedding
               FROM mem_tree_summaries
              WHERE tree_id = ?1 AND level = ?2 AND deleted = 0
              ORDER BY sealed_at_ms ASC",
        )?;
        let rows = stmt
            .query_map(params![tree_id, level], row_to_summary)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect summaries")?;
        Ok(rows)
    })
}

/// Count summaries in a tree (diagnostic helper).
pub fn count_summaries(config: &Config, tree_id: &str) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM mem_tree_summaries
                  WHERE tree_id = ?1 AND deleted = 0",
                params![tree_id],
                |r| r.get(0),
            )
            .context("count summaries query")?;
        Ok(n.max(0) as u64)
    })
}

fn row_to_summary(row: &rusqlite::Row<'_>) -> rusqlite::Result<SummaryNode> {
    let id: String = row.get(0)?;
    let tree_id: String = row.get(1)?;
    let tree_kind_s: String = row.get(2)?;
    let level: i64 = row.get(3)?;
    let parent_id: Option<String> = row.get(4)?;
    let child_ids_json: String = row.get(5)?;
    let content: String = row.get(6)?;
    let token_count: i64 = row.get(7)?;
    let entities_json: String = row.get(8)?;
    let topics_json: String = row.get(9)?;
    let trs_ms: i64 = row.get(10)?;
    let tre_ms: i64 = row.get(11)?;
    let score: f64 = row.get(12)?;
    let sealed_ms: i64 = row.get(13)?;
    let deleted: i64 = row.get(14)?;
    let embedding_blob: Option<Vec<u8>> = row.get(15)?;

    let tree_kind = TreeKind::parse(&tree_kind_s).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, e.into())
    })?;
    let child_ids: Vec<String> = serde_json::from_str(&child_ids_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let entities: Vec<String> = serde_json::from_str(&entities_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let topics: Vec<String> = serde_json::from_str(&topics_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(9, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let embedding =
        decode_optional_blob(embedding_blob, &format!("summary_id={id}")).map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                15,
                rusqlite::types::Type::Blob,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    e.to_string(),
                )),
            )
        })?;

    Ok(SummaryNode {
        id,
        tree_id,
        tree_kind,
        level: level.max(0) as u32,
        parent_id,
        child_ids,
        content,
        token_count: token_count.max(0) as u32,
        entities,
        topics,
        time_range_start: ms_to_utc(trs_ms)?,
        time_range_end: ms_to_utc(tre_ms)?,
        score: score as f32,
        sealed_at: ms_to_utc(sealed_ms)?,
        deleted: deleted != 0,
        embedding,
    })
}

// ── Buffers ─────────────────────────────────────────────────────────────

/// Read the current buffer at `(tree_id, level)` or return an empty one.
pub fn get_buffer(config: &Config, tree_id: &str, level: u32) -> Result<Buffer> {
    with_connection(config, |conn| get_buffer_conn(conn, tree_id, level))
}

pub(crate) fn get_buffer_conn(conn: &Connection, tree_id: &str, level: u32) -> Result<Buffer> {
    let mut stmt = conn.prepare(
        "SELECT tree_id, level, item_ids_json, token_sum, oldest_at_ms
           FROM mem_tree_buffers WHERE tree_id = ?1 AND level = ?2",
    )?;
    let row = stmt
        .query_row(params![tree_id, level], row_to_buffer)
        .optional()
        .context("Failed to query buffer")?;
    Ok(row.unwrap_or_else(|| Buffer::empty(tree_id, level)))
}

/// Upsert a buffer row.
pub(crate) fn upsert_buffer_tx(tx: &Transaction<'_>, buf: &Buffer) -> Result<()> {
    let now_ms = Utc::now().timestamp_millis();
    tx.execute(
        "INSERT INTO mem_tree_buffers (
            tree_id, level, item_ids_json, token_sum, oldest_at_ms, updated_at_ms
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(tree_id, level) DO UPDATE SET
            item_ids_json = excluded.item_ids_json,
            token_sum = excluded.token_sum,
            oldest_at_ms = excluded.oldest_at_ms,
            updated_at_ms = excluded.updated_at_ms",
        params![
            buf.tree_id,
            buf.level,
            serde_json::to_string(&buf.item_ids)?,
            buf.token_sum,
            buf.oldest_at.map(|t| t.timestamp_millis()),
            now_ms,
        ],
    )
    .with_context(|| {
        format!(
            "Failed to upsert buffer tree_id={} level={}",
            buf.tree_id, buf.level
        )
    })?;
    Ok(())
}

/// Reset a buffer at `(tree_id, level)` to empty. Used at seal time: the
/// items move into a summary row and the buffer is cleared in the same tx.
pub(crate) fn clear_buffer_tx(tx: &Transaction<'_>, tree_id: &str, level: u32) -> Result<()> {
    let empty = Buffer::empty(tree_id, level);
    upsert_buffer_tx(tx, &empty)
}

/// List stale **L0** buffers ordered by `oldest_at_ms ASC`. Used by the
/// time-based flush pass.
///
/// Only L0 (raw-leaf) buffers are returned. Force-sealing an L≥1 buffer
/// that hasn't met the [`SUMMARY_FANOUT`](super::types::SUMMARY_FANOUT)
/// gate produces a degenerate single-child summary that wraps exactly the
/// same content as its only child — repeated flush cycles cascade these
/// no-op promotions up the tree and collapse the upper levels into a
/// 1:1:1 chain. Upper-level buffers must seal only when their fan-in
/// gate is naturally met.
pub fn list_stale_buffers(config: &Config, older_than: DateTime<Utc>) -> Result<Vec<Buffer>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT tree_id, level, item_ids_json, token_sum, oldest_at_ms
               FROM mem_tree_buffers
              WHERE oldest_at_ms IS NOT NULL
                AND oldest_at_ms <= ?1
                AND level = 0
              ORDER BY oldest_at_ms ASC",
        )?;
        let rows = stmt
            .query_map(params![older_than.timestamp_millis()], row_to_buffer)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .context("Failed to collect stale buffers")?;
        Ok(rows)
    })
}

fn row_to_buffer(row: &rusqlite::Row<'_>) -> rusqlite::Result<Buffer> {
    let tree_id: String = row.get(0)?;
    let level: i64 = row.get(1)?;
    let item_ids_json: String = row.get(2)?;
    let token_sum: i64 = row.get(3)?;
    let oldest_ms: Option<i64> = row.get(4)?;

    let item_ids: Vec<String> = serde_json::from_str(&item_ids_json).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let oldest_at = oldest_ms.map(ms_to_utc).transpose()?;
    Ok(Buffer {
        tree_id,
        level: level.max(0) as u32,
        item_ids,
        token_sum,
        oldest_at,
    })
}

fn pack_embedding_blob(embedding: &[f32]) -> Vec<u8> {
    embedding.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn decode_signature_blob(blob: Option<Vec<u8>>, dim: i64, label: &str) -> Result<Option<Vec<f32>>> {
    let Some(bytes) = blob else {
        return Ok(None);
    };
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
            "summary embedding dimension mismatch: dim column says {dim}, blob contains {} floats",
            floats.len()
        );
    }
    Ok(Some(floats))
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
