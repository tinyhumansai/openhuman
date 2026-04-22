//! SQLite persistence for topic-tree-specific state (#709 Phase 3c).
//!
//! The only new table owned here is `mem_tree_entity_hotness` — the
//! per-entity counter block driving lazy materialisation. Tree rows and
//! summary nodes are reused from [`super::super::source_tree::store`] via
//! the shared `mem_tree_trees` / `mem_tree_summaries` / `mem_tree_buffers`
//! tables, which already carry a `kind` column that discriminates
//! `source` from `topic`. No schema additions for those tables in Phase
//! 3c — only the new hotness table.
//!
//! Schema for `mem_tree_entity_hotness` is declared in
//! [`super::super::store::SCHEMA`] (the sibling Phase 1 store file) so
//! migrations all run through the same `with_connection` entry point.

use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{params, OptionalExtension};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::store::with_connection;
use crate::openhuman::memory::tree::topic_tree::types::HotnessCounters;

/// Fetch the hotness row for `entity_id`, or `None` if the entity has
/// never been seen. Callers usually want [`get_or_fresh`] instead.
pub fn get(config: &Config, entity_id: &str) -> Result<Option<HotnessCounters>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT entity_id, mention_count_30d, distinct_sources, last_seen_ms,
                    query_hits_30d, graph_centrality, ingests_since_check,
                    last_hotness, last_updated_ms
               FROM mem_tree_entity_hotness WHERE entity_id = ?1",
        )?;
        let row = stmt
            .query_row(params![entity_id], row_to_counters)
            .optional()
            .context("failed to query mem_tree_entity_hotness")?;
        Ok(row)
    })
}

/// Fetch the hotness row, or return a fresh (all-zero) row if the entity
/// has never been seen. The fresh row is NOT persisted — callers must
/// [`upsert`] it explicitly after bumping counters.
pub fn get_or_fresh(config: &Config, entity_id: &str) -> Result<HotnessCounters> {
    match get(config, entity_id)? {
        Some(c) => Ok(c),
        None => Ok(HotnessCounters::fresh(
            entity_id,
            Utc::now().timestamp_millis(),
        )),
    }
}

/// Upsert the full counter row. Idempotent on `entity_id`.
pub fn upsert(config: &Config, counters: &HotnessCounters) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT INTO mem_tree_entity_hotness (
                entity_id, mention_count_30d, distinct_sources, last_seen_ms,
                query_hits_30d, graph_centrality, ingests_since_check,
                last_hotness, last_updated_ms
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(entity_id) DO UPDATE SET
                mention_count_30d = excluded.mention_count_30d,
                distinct_sources  = excluded.distinct_sources,
                last_seen_ms      = excluded.last_seen_ms,
                query_hits_30d    = excluded.query_hits_30d,
                graph_centrality  = excluded.graph_centrality,
                ingests_since_check = excluded.ingests_since_check,
                last_hotness      = excluded.last_hotness,
                last_updated_ms   = excluded.last_updated_ms",
            params![
                counters.entity_id,
                counters.mention_count_30d,
                counters.distinct_sources,
                counters.last_seen_ms,
                counters.query_hits_30d,
                counters.graph_centrality,
                counters.ingests_since_check,
                counters.last_hotness,
                counters.last_updated_ms,
            ],
        )
        .with_context(|| {
            format!(
                "failed to upsert mem_tree_entity_hotness for {}",
                counters.entity_id
            )
        })?;
        Ok(())
    })
}

/// Count `(node_id) → DISTINCT tree_id` in the entity index for `entity_id`.
/// Used by the curator to refresh `distinct_sources` during the periodic
/// hotness recompute without rescanning every chunk.
pub fn distinct_sources_for(config: &Config, entity_id: &str) -> Result<u32> {
    with_connection(config, |conn| {
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(DISTINCT tree_id)
                   FROM mem_tree_entity_index
                  WHERE entity_id = ?1 AND tree_id IS NOT NULL",
                params![entity_id],
                |r| r.get(0),
            )
            .context("failed to count distinct sources")?;
        Ok(n.max(0) as u32)
    })
}

/// Test / diagnostic helper.
pub fn count(config: &Config) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM mem_tree_entity_hotness", [], |r| {
                r.get(0)
            })
            .context("failed to count mem_tree_entity_hotness")?;
        Ok(n.max(0) as u64)
    })
}

fn row_to_counters(row: &rusqlite::Row<'_>) -> rusqlite::Result<HotnessCounters> {
    Ok(HotnessCounters {
        entity_id: row.get(0)?,
        mention_count_30d: row.get::<_, i64>(1)?.max(0) as u32,
        distinct_sources: row.get::<_, i64>(2)?.max(0) as u32,
        last_seen_ms: row.get(3)?,
        query_hits_30d: row.get::<_, i64>(4)?.max(0) as u32,
        graph_centrality: row.get(5)?,
        ingests_since_check: row.get::<_, i64>(6)?.max(0) as u32,
        last_hotness: row.get(7)?,
        last_updated_ms: row.get(8)?,
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

    #[test]
    fn get_missing_is_none() {
        let (_tmp, cfg) = test_config();
        assert!(get(&cfg, "email:alice@example.com").unwrap().is_none());
    }

    #[test]
    fn get_or_fresh_returns_zero_row() {
        let (_tmp, cfg) = test_config();
        let c = get_or_fresh(&cfg, "email:alice@example.com").unwrap();
        assert_eq!(c.entity_id, "email:alice@example.com");
        assert_eq!(c.mention_count_30d, 0);
        assert_eq!(c.distinct_sources, 0);
        assert!(c.last_hotness.is_none());
        // Not persisted — still zero rows in the table.
        assert_eq!(count(&cfg).unwrap(), 0);
    }

    #[test]
    fn upsert_round_trip() {
        let (_tmp, cfg) = test_config();
        let c = HotnessCounters {
            entity_id: "email:alice@example.com".into(),
            mention_count_30d: 12,
            distinct_sources: 3,
            last_seen_ms: Some(1_700_000_000_000),
            query_hits_30d: 2,
            graph_centrality: Some(0.25),
            ingests_since_check: 40,
            last_hotness: Some(9.5),
            last_updated_ms: 1_700_000_123_000,
        };
        upsert(&cfg, &c).unwrap();
        let got = get(&cfg, &c.entity_id).unwrap().unwrap();
        assert_eq!(got, c);
        assert_eq!(count(&cfg).unwrap(), 1);
    }

    #[test]
    fn upsert_is_idempotent_and_updates_fields() {
        let (_tmp, cfg) = test_config();
        let mut c = HotnessCounters::fresh("email:alice@example.com", 0);
        c.mention_count_30d = 1;
        upsert(&cfg, &c).unwrap();
        c.mention_count_30d = 99;
        c.last_updated_ms = 500;
        upsert(&cfg, &c).unwrap();
        assert_eq!(count(&cfg).unwrap(), 1);
        let got = get(&cfg, "email:alice@example.com").unwrap().unwrap();
        assert_eq!(got.mention_count_30d, 99);
        assert_eq!(got.last_updated_ms, 500);
    }

    #[test]
    fn distinct_sources_counts_trees() {
        use crate::openhuman::memory::tree::score::extract::EntityKind;
        use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
        use crate::openhuman::memory::tree::score::store::index_entity;
        let (_tmp, cfg) = test_config();
        let e = CanonicalEntity {
            canonical_id: "email:alice@example.com".into(),
            kind: EntityKind::Email,
            surface: "alice@example.com".into(),
            span_start: 0,
            span_end: 17,
            score: 1.0,
        };
        index_entity(&cfg, &e, "chunk-1", "leaf", 1000, Some("source:slack")).unwrap();
        index_entity(&cfg, &e, "chunk-2", "leaf", 2000, Some("source:gmail")).unwrap();
        index_entity(&cfg, &e, "chunk-3", "leaf", 3000, Some("source:slack")).unwrap();
        // 3 rows but only 2 distinct tree_ids.
        let n = distinct_sources_for(&cfg, "email:alice@example.com").unwrap();
        assert_eq!(n, 2);
    }

    #[test]
    fn distinct_sources_ignores_null_tree_id() {
        use crate::openhuman::memory::tree::score::extract::EntityKind;
        use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
        use crate::openhuman::memory::tree::score::store::index_entity;
        let (_tmp, cfg) = test_config();
        let e = CanonicalEntity {
            canonical_id: "email:alice@example.com".into(),
            kind: EntityKind::Email,
            surface: "alice@example.com".into(),
            span_start: 0,
            span_end: 17,
            score: 1.0,
        };
        // tree_id = None — should not count toward distinct_sources.
        index_entity(&cfg, &e, "chunk-1", "leaf", 1000, None).unwrap();
        index_entity(&cfg, &e, "chunk-2", "leaf", 2000, Some("source:slack")).unwrap();
        let n = distinct_sources_for(&cfg, "email:alice@example.com").unwrap();
        assert_eq!(n, 1);
    }
}
