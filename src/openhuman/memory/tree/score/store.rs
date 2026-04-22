//! Persistence for Phase 2 artefacts (#708):
//!
//! - `mem_tree_score` — per-chunk score rationale (which signals fired, why
//!   dropped/kept)
//! - `mem_tree_entity_index` — inverted index `entity_id → node_id` so
//!   retrieval can resolve entity-scoped queries in O(lookup)
//!
//! Schema is declared in `memory/tree/store.rs::SCHEMA`; this file only
//! owns the CRUD operations.

use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::score::extract::EntityKind;
use crate::openhuman::memory::tree::score::resolver::CanonicalEntity;
use crate::openhuman::memory::tree::score::signals::ScoreSignals;
use crate::openhuman::memory::tree::store::with_connection;

/// Serialized per-chunk score rationale. Mirrors the `mem_tree_score` row.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScoreRow {
    pub chunk_id: String,
    pub total: f32,
    pub signals: ScoreSignals,
    pub dropped: bool,
    pub reason: Option<String>,
    pub computed_at_ms: i64,
}

/// Upsert one score rationale row, replacing any existing entry for `chunk_id`.
pub fn upsert_score(config: &Config, row: &ScoreRow) -> Result<()> {
    with_connection(config, |conn| {
        upsert_score_on_connection(conn, row)?;
        Ok(())
    })
}

pub(crate) fn upsert_score_tx(tx: &Transaction<'_>, row: &ScoreRow) -> Result<()> {
    tx.execute(
        SCORE_UPSERT_SQL,
        params![
            row.chunk_id,
            row.total,
            row.signals.token_count,
            row.signals.unique_words,
            row.signals.metadata_weight,
            row.signals.source_weight,
            row.signals.interaction,
            row.signals.entity_density,
            i32::from(row.dropped),
            row.reason,
            row.computed_at_ms,
        ],
    )?;
    Ok(())
}

const SCORE_UPSERT_SQL: &str = "INSERT OR REPLACE INTO mem_tree_score (
    chunk_id, total,
    token_count_signal, unique_words_signal,
    metadata_weight, source_weight, interaction_weight, entity_density,
    dropped, reason, computed_at_ms
 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)";

fn upsert_score_on_connection(conn: &Connection, row: &ScoreRow) -> Result<()> {
    conn.execute(
        SCORE_UPSERT_SQL,
        params![
            row.chunk_id,
            row.total,
            row.signals.token_count,
            row.signals.unique_words,
            row.signals.metadata_weight,
            row.signals.source_weight,
            row.signals.interaction,
            row.signals.entity_density,
            i32::from(row.dropped),
            row.reason,
            row.computed_at_ms,
        ],
    )?;
    Ok(())
}

/// Fetch one chunk's score rationale.
pub fn get_score(config: &Config, chunk_id: &str) -> Result<Option<ScoreRow>> {
    with_connection(config, |conn| {
        conn.query_row(
            "SELECT chunk_id, total,
                    token_count_signal, unique_words_signal,
                    metadata_weight, source_weight, interaction_weight, entity_density,
                    dropped, reason, computed_at_ms
             FROM mem_tree_score WHERE chunk_id = ?1",
            params![chunk_id],
            |row| {
                Ok(ScoreRow {
                    chunk_id: row.get(0)?,
                    total: row.get(1)?,
                    signals: ScoreSignals {
                        token_count: row.get(2)?,
                        unique_words: row.get(3)?,
                        metadata_weight: row.get(4)?,
                        source_weight: row.get(5)?,
                        interaction: row.get(6)?,
                        entity_density: row.get(7)?,
                    },
                    dropped: row.get::<_, i32>(8)? != 0,
                    reason: row.get(9)?,
                    computed_at_ms: row.get(10)?,
                })
            },
        )
        .optional()
        .map_err(anyhow::Error::from)
    })
}

/// Index one (entity, chunk) association.
///
/// Idempotent on the composite primary key `(entity_id, node_id)` so
/// re-indexing the same association is a no-op update.
pub fn index_entity(
    config: &Config,
    entity: &CanonicalEntity,
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<()> {
    with_connection(config, |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO mem_tree_entity_index (
                entity_id, node_id, node_kind, entity_kind, surface,
                score, timestamp_ms, tree_id
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                entity.canonical_id,
                node_id,
                node_kind,
                entity.kind.as_str(),
                entity.surface,
                entity.score,
                timestamp_ms,
                tree_id,
            ],
        )?;
        Ok(())
    })
}

/// Batch index all entities extracted from a chunk.
pub fn index_entities(
    config: &Config,
    entities: &[CanonicalEntity],
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    if entities.is_empty() {
        return Ok(0);
    }
    with_connection(config, |conn| {
        let tx = conn.unchecked_transaction()?;
        {
            let mut stmt = tx.prepare(
                "INSERT OR REPLACE INTO mem_tree_entity_index (
                    entity_id, node_id, node_kind, entity_kind, surface,
                    score, timestamp_ms, tree_id
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )?;
            for e in entities {
                stmt.execute(params![
                    e.canonical_id,
                    node_id,
                    node_kind,
                    e.kind.as_str(),
                    e.surface,
                    e.score,
                    timestamp_ms,
                    tree_id,
                ])?;
            }
        }
        tx.commit()?;
        Ok(entities.len())
    })
}

/// Remove all entity-index rows for a given node. Used before re-indexing
/// a re-scored chunk so entities dropped from the new extraction don't leak
/// through as stale `INSERT OR REPLACE` never deletes.
pub fn clear_entity_index_for_node(config: &Config, node_id: &str) -> Result<usize> {
    with_connection(config, |conn| {
        let n = conn.execute(
            "DELETE FROM mem_tree_entity_index WHERE node_id = ?1",
            params![node_id],
        )?;
        Ok(n)
    })
}

pub(crate) fn clear_entity_index_for_node_tx(tx: &Transaction<'_>, node_id: &str) -> Result<usize> {
    let n = tx.execute(
        "DELETE FROM mem_tree_entity_index WHERE node_id = ?1",
        params![node_id],
    )?;
    Ok(n)
}

pub(crate) fn index_entities_tx(
    tx: &Transaction<'_>,
    entities: &[CanonicalEntity],
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    if entities.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT OR REPLACE INTO mem_tree_entity_index (
            entity_id, node_id, node_kind, entity_kind, surface,
            score, timestamp_ms, tree_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for e in entities {
        stmt.execute(params![
            e.canonical_id,
            node_id,
            node_kind,
            e.kind.as_str(),
            e.surface,
            e.score,
            timestamp_ms,
            tree_id,
        ])?;
    }
    Ok(entities.len())
}

/// Result row from [`lookup_entity`].
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EntityHit {
    pub entity_id: String,
    pub node_id: String,
    pub node_kind: String,
    pub entity_kind: EntityKind,
    pub surface: String,
    pub score: f32,
    pub timestamp_ms: i64,
    pub tree_id: Option<String>,
}

/// Find all nodes indexed against `entity_id`, newest first.
pub fn lookup_entity(
    config: &Config,
    entity_id: &str,
    limit: Option<usize>,
) -> Result<Vec<EntityHit>> {
    // Clamp to i64::MAX before casting so callers can't wrap a large usize
    // into a negative LIMIT and bypass it.
    let limit = limit.unwrap_or(100).min(i64::MAX as usize) as i64;
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT entity_id, node_id, node_kind, entity_kind, surface,
                    score, timestamp_ms, tree_id
             FROM mem_tree_entity_index
             WHERE entity_id = ?1
             ORDER BY timestamp_ms DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(params![entity_id, limit], |row| {
                let kind_s: String = row.get(3)?;
                let entity_kind = EntityKind::parse(&kind_s).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Text,
                        e.into(),
                    )
                })?;
                Ok(EntityHit {
                    entity_id: row.get(0)?,
                    node_id: row.get(1)?,
                    node_kind: row.get(2)?,
                    entity_kind,
                    surface: row.get(4)?,
                    score: row.get(5)?,
                    timestamp_ms: row.get(6)?,
                    tree_id: row.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// Count rows in the entity index (for tests / diagnostics).
pub fn count_entity_index(config: &Config) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_entity_index", [], |r| {
            r.get(0)
        })?;
        Ok(n.max(0) as u64)
    })
}

/// Count score rows (for tests / diagnostics).
pub fn count_scores(config: &Config) -> Result<u64> {
    with_connection(config, |conn| {
        let n: i64 = conn.query_row("SELECT COUNT(*) FROM mem_tree_score", [], |r| r.get(0))?;
        Ok(n.max(0) as u64)
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

    fn sample_row(id: &str, dropped: bool) -> ScoreRow {
        ScoreRow {
            chunk_id: id.to_string(),
            total: 0.7,
            signals: ScoreSignals {
                token_count: 1.0,
                unique_words: 0.8,
                metadata_weight: 0.9,
                source_weight: 0.5,
                interaction: 0.6,
                entity_density: 0.3,
            },
            dropped,
            reason: if dropped {
                Some("below threshold".into())
            } else {
                None
            },
            computed_at_ms: 1_700_000_000_000,
        }
    }

    fn sample_entity(id: &str) -> CanonicalEntity {
        CanonicalEntity {
            canonical_id: format!("email:{id}"),
            kind: EntityKind::Email,
            surface: format!("{id}@example.com"),
            span_start: 0,
            span_end: (id.len() + 12) as u32,
            score: 1.0,
        }
    }

    #[test]
    fn upsert_then_get_score() {
        let (_tmp, cfg) = test_config();
        let row = sample_row("c1", false);
        upsert_score(&cfg, &row).unwrap();
        let got = get_score(&cfg, "c1").unwrap().expect("row exists");
        assert_eq!(got.chunk_id, row.chunk_id);
        assert!((got.total - row.total).abs() < 1e-6);
        assert_eq!(got.dropped, row.dropped);
        assert_eq!(got.reason, row.reason);
        assert_eq!(got.computed_at_ms, row.computed_at_ms);
        assert!((got.signals.token_count - row.signals.token_count).abs() < 1e-6);
    }

    #[test]
    fn upsert_score_idempotent() {
        let (_tmp, cfg) = test_config();
        let r = sample_row("c1", false);
        upsert_score(&cfg, &r).unwrap();
        upsert_score(&cfg, &r).unwrap();
        assert_eq!(count_scores(&cfg).unwrap(), 1);
    }

    #[test]
    fn dropped_flag_persists() {
        let (_tmp, cfg) = test_config();
        let r = sample_row("c1", true);
        upsert_score(&cfg, &r).unwrap();
        let got = get_score(&cfg, "c1").unwrap().unwrap();
        assert!(got.dropped);
        assert_eq!(got.reason.as_deref(), Some("below threshold"));
    }

    #[test]
    fn get_missing_score_is_none() {
        let (_tmp, cfg) = test_config();
        assert!(get_score(&cfg, "missing").unwrap().is_none());
    }

    #[test]
    fn index_and_lookup_entity() {
        let (_tmp, cfg) = test_config();
        let e = sample_entity("alice");
        index_entity(&cfg, &e, "chunk-1", "leaf", 1000, Some("source:chat")).unwrap();
        index_entity(&cfg, &e, "chunk-2", "leaf", 2000, Some("source:chat")).unwrap();

        let hits = lookup_entity(&cfg, "email:alice", None).unwrap();
        assert_eq!(hits.len(), 2);
        // newest first
        assert_eq!(hits[0].node_id, "chunk-2");
        assert_eq!(hits[1].node_id, "chunk-1");
    }

    #[test]
    fn index_batch() {
        let (_tmp, cfg) = test_config();
        let entities = vec![sample_entity("a"), sample_entity("b"), sample_entity("c")];
        let n = index_entities(&cfg, &entities, "chunk-1", "leaf", 1000, None).unwrap();
        assert_eq!(n, 3);
        assert_eq!(count_entity_index(&cfg).unwrap(), 3);
    }

    #[test]
    fn clear_entity_index_drops_stale_rows() {
        let (_tmp, cfg) = test_config();
        let a = sample_entity("a");
        let b = sample_entity("b");
        index_entities(&cfg, &[a.clone(), b], "chunk-1", "leaf", 1000, None).unwrap();
        assert_eq!(count_entity_index(&cfg).unwrap(), 2);

        // Simulate a re-score that only keeps entity "a".
        let cleared = clear_entity_index_for_node(&cfg, "chunk-1").unwrap();
        assert_eq!(cleared, 2);
        index_entities(&cfg, &[a], "chunk-1", "leaf", 1000, None).unwrap();

        let hits = lookup_entity(&cfg, "email:b", None).unwrap();
        assert!(hits.is_empty(), "stale entity should be removed");
        let hits = lookup_entity(&cfg, "email:a", None).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn index_idempotent_per_entity_node_pair() {
        let (_tmp, cfg) = test_config();
        let e = sample_entity("alice");
        index_entity(&cfg, &e, "chunk-1", "leaf", 1000, None).unwrap();
        index_entity(&cfg, &e, "chunk-1", "leaf", 1000, None).unwrap();
        assert_eq!(count_entity_index(&cfg).unwrap(), 1);
    }

    #[test]
    fn lookup_limit_respected() {
        let (_tmp, cfg) = test_config();
        let e = sample_entity("alice");
        for i in 0..5 {
            index_entity(
                &cfg,
                &e,
                &format!("chunk-{i}"),
                "leaf",
                1000 + i as i64,
                None,
            )
            .unwrap();
        }
        let hits = lookup_entity(&cfg, "email:alice", Some(2)).unwrap();
        assert_eq!(hits.len(), 2);
    }
}
