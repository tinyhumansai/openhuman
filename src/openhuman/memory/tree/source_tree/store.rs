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
use crate::openhuman::memory::tree::score::embed::{decode_optional_blob, pack_checked};
use crate::openhuman::memory::tree::source_tree::types::{
    Buffer, SummaryNode, Tree, TreeKind, TreeStatus,
};
use crate::openhuman::memory::tree::store::with_connection;

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
pub(crate) fn insert_summary_tx(tx: &Transaction<'_>, node: &SummaryNode) -> Result<()> {
    let embedding_blob: Option<Vec<u8>> = match node.embedding.as_deref() {
        Some(v) => Some(
            pack_checked(v)
                .with_context(|| format!("Failed to pack embedding for summary id={}", node.id))?,
        ),
        None => None,
    };
    tx.execute(
        "INSERT OR IGNORE INTO mem_tree_summaries (
            id, tree_id, tree_kind, level, parent_id,
            child_ids_json, content, token_count,
            entities_json, topics_json,
            time_range_start_ms, time_range_end_ms,
            score, sealed_at_ms, deleted, embedding
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            node.id,
            node.tree_id,
            node.tree_kind.as_str(),
            node.level,
            node.parent_id,
            serde_json::to_string(&node.child_ids)?,
            node.content,
            node.token_count,
            serde_json::to_string(&node.entities)?,
            serde_json::to_string(&node.topics)?,
            node.time_range_start.timestamp_millis(),
            node.time_range_end.timestamp_millis(),
            node.score,
            node.sealed_at.timestamp_millis(),
            node.deleted as i64,
            embedding_blob,
        ],
    )
    .with_context(|| format!("Failed to insert summary id={}", node.id))?;
    Ok(())
}

/// Set (or overwrite) the embedding for an existing summary row.
/// Exposed for a future backfill helper — not called by ingest/seal
/// today. Returns the number of rows updated (0 if the id is unknown).
pub fn set_summary_embedding(
    config: &Config,
    summary_id: &str,
    embedding: &[f32],
) -> Result<usize> {
    let blob = pack_checked(embedding)
        .with_context(|| format!("Failed to pack embedding for summary id={summary_id}"))?;
    with_connection(config, |conn| {
        let changed = conn.execute(
            "UPDATE mem_tree_summaries SET embedding = ?1 WHERE id = ?2",
            params![blob, summary_id],
        )?;
        if changed == 0 {
            log::warn!(
                "[source_tree::store] set_summary_embedding: no row for summary_id={summary_id}"
            );
        }
        Ok(changed)
    })
}

/// Fetch a summary's embedding, decoding the stored little-endian `f32`
/// blob. Returns `Ok(None)` if the summary doesn't exist OR if it exists
/// but has a NULL embedding (legacy / pre-Phase-4 rows).
pub fn get_summary_embedding(config: &Config, summary_id: &str) -> Result<Option<Vec<f32>>> {
    with_connection(config, |conn| {
        let blob: Option<Option<Vec<u8>>> = conn
            .query_row(
                "SELECT embedding FROM mem_tree_summaries WHERE id = ?1",
                params![summary_id],
                |r| r.get::<_, Option<Vec<u8>>>(0),
            )
            .optional()?;
        match blob {
            None => Ok(None),
            Some(inner) => decode_optional_blob(inner, &format!("summary_id={summary_id}")),
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

/// List all non-empty buffers ordered by `oldest_at_ms ASC`. Used by the
/// time-based flush pass.
pub fn list_stale_buffers(config: &Config, older_than: DateTime<Utc>) -> Result<Vec<Buffer>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT tree_id, level, item_ids_json, token_sum, oldest_at_ms
               FROM mem_tree_buffers
              WHERE oldest_at_ms IS NOT NULL
                AND oldest_at_ms <= ?1
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

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
