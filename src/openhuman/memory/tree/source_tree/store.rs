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

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

use crate::openhuman::config::Config;
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
pub(crate) fn insert_summary_tx(tx: &Transaction<'_>, node: &SummaryNode) -> Result<()> {
    tx.execute(
        "INSERT OR IGNORE INTO mem_tree_summaries (
            id, tree_id, tree_kind, level, parent_id,
            child_ids_json, content, token_count,
            entities_json, topics_json,
            time_range_start_ms, time_range_end_ms,
            score, sealed_at_ms, deleted
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
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
        ],
    )
    .with_context(|| format!("Failed to insert summary id={}", node.id))?;
    Ok(())
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
                    score, sealed_at_ms, deleted
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
                    score, sealed_at_ms, deleted
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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn sample_tree(id: &str, scope: &str) -> Tree {
        Tree {
            id: id.to_string(),
            kind: TreeKind::Source,
            scope: scope.to_string(),
            root_id: None,
            max_level: 0,
            status: TreeStatus::Active,
            created_at: Utc.timestamp_millis_opt(1_700_000_000_000).unwrap(),
            last_sealed_at: None,
        }
    }

    fn sample_summary(id: &str, tree_id: &str, level: u32) -> SummaryNode {
        let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        SummaryNode {
            id: id.to_string(),
            tree_id: tree_id.to_string(),
            tree_kind: TreeKind::Source,
            level,
            parent_id: None,
            child_ids: vec!["leaf-a".into(), "leaf-b".into()],
            content: "seal content".into(),
            token_count: 100,
            entities: vec!["entity:alice".into()],
            topics: vec!["#launch".into()],
            time_range_start: ts,
            time_range_end: ts,
            score: 0.75,
            sealed_at: ts,
            deleted: false,
        }
    }

    #[test]
    fn tree_round_trip() {
        let (_tmp, cfg) = test_config();
        let t = sample_tree("tree-1", "slack:#eng");
        insert_tree(&cfg, &t).unwrap();
        let got = get_tree(&cfg, "tree-1").unwrap().unwrap();
        assert_eq!(got, t);
        let by_scope = get_tree_by_scope(&cfg, TreeKind::Source, "slack:#eng")
            .unwrap()
            .unwrap();
        assert_eq!(by_scope.id, "tree-1");
    }

    #[test]
    fn duplicate_scope_fails() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("t1", "slack:#eng")).unwrap();
        let dup = sample_tree("t2", "slack:#eng");
        assert!(insert_tree(&cfg, &dup).is_err());
    }

    #[test]
    fn summary_insert_and_fetch() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
        let node = sample_summary("sum-1", "tree-1", 1);
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            insert_summary_tx(&tx, &node)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let got = get_summary(&cfg, "sum-1").unwrap().unwrap();
        assert_eq!(got, node);
        let at_level = list_summaries_at_level(&cfg, "tree-1", 1).unwrap();
        assert_eq!(at_level.len(), 1);
        assert_eq!(count_summaries(&cfg, "tree-1").unwrap(), 1);
    }

    #[test]
    fn summary_insert_is_idempotent_on_id() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
        let node = sample_summary("sum-1", "tree-1", 1);
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            insert_summary_tx(&tx, &node)?;
            insert_summary_tx(&tx, &node)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        assert_eq!(count_summaries(&cfg, "tree-1").unwrap(), 1);
    }

    #[test]
    fn buffer_upsert_and_clear() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
        let ts = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let buf = Buffer {
            tree_id: "tree-1".into(),
            level: 0,
            item_ids: vec!["leaf-a".into(), "leaf-b".into()],
            token_sum: 500,
            oldest_at: Some(ts),
        };
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_buffer_tx(&tx, &buf)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let got = get_buffer(&cfg, "tree-1", 0).unwrap();
        assert_eq!(got, buf);

        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            clear_buffer_tx(&tx, "tree-1", 0)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let cleared = get_buffer(&cfg, "tree-1", 0).unwrap();
        assert!(cleared.is_empty());
        assert_eq!(cleared.token_sum, 0);
        assert!(cleared.oldest_at.is_none());
    }

    #[test]
    fn get_buffer_returns_empty_when_missing() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
        let got = get_buffer(&cfg, "tree-1", 0).unwrap();
        assert!(got.is_empty());
        assert_eq!(got.tree_id, "tree-1");
    }

    #[test]
    fn update_tree_after_seal_persists() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
        let sealed_at = Utc.timestamp_millis_opt(1_700_000_123_000).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            update_tree_after_seal_tx(&tx, "tree-1", "sum-1", 1, sealed_at)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let got = get_tree(&cfg, "tree-1").unwrap().unwrap();
        assert_eq!(got.root_id.as_deref(), Some("sum-1"));
        assert_eq!(got.max_level, 1);
        assert_eq!(got.last_sealed_at, Some(sealed_at));
    }

    #[test]
    fn list_stale_buffers_orders_by_age() {
        let (_tmp, cfg) = test_config();
        insert_tree(&cfg, &sample_tree("tree-1", "slack:#eng")).unwrap();
        let t0 = Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let t1 = Utc.timestamp_millis_opt(1_700_000_010_000).unwrap();
        let t2 = Utc.timestamp_millis_opt(1_700_000_020_000).unwrap();
        with_connection(&cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_buffer_tx(
                &tx,
                &Buffer {
                    tree_id: "tree-1".into(),
                    level: 0,
                    item_ids: vec!["a".into()],
                    token_sum: 10,
                    oldest_at: Some(t0),
                },
            )?;
            upsert_buffer_tx(
                &tx,
                &Buffer {
                    tree_id: "tree-1".into(),
                    level: 1,
                    item_ids: vec!["b".into()],
                    token_sum: 20,
                    oldest_at: Some(t1),
                },
            )?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
        let stale = list_stale_buffers(&cfg, t2).unwrap();
        assert_eq!(stale.len(), 2);
        assert_eq!(stale[0].level, 0);
        assert_eq!(stale[1].level, 1);
        // Filter out the first: only level-1 should come back.
        let only_later = list_stale_buffers(&cfg, t0).unwrap();
        assert_eq!(only_later.len(), 1);
        assert_eq!(only_later[0].level, 0);
    }
}
