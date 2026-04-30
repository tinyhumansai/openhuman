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
    /// One-line LLM-supplied explanation for the importance rating; useful
    /// for tuning prompts and thresholds. The numeric value lives on
    /// `signals.llm_importance`.
    #[serde(default)]
    pub llm_importance_reason: Option<String>,
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
            row.signals.llm_importance,
            row.llm_importance_reason,
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
    llm_importance, llm_importance_reason,
    dropped, reason, computed_at_ms
 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)";

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
            row.signals.llm_importance,
            row.llm_importance_reason,
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
                    llm_importance, llm_importance_reason,
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
                        llm_importance: row.get::<_, Option<f32>>(8)?.unwrap_or(0.0),
                    },
                    llm_importance_reason: row.get::<_, Option<String>>(9)?,
                    dropped: row.get::<_, i32>(10)? != 0,
                    reason: row.get(11)?,
                    computed_at_ms: row.get(12)?,
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

/// Index summary-node entities by canonical id only. Summary-level entity
/// metadata is LLM-derived (Phase 3a #709) — the summariser emits a
/// curated list of canonical ids without per-occurrence span/surface data.
///
/// Writes the kind prefix (everything before the first `:`) into the
/// `entity_kind` column so [`lookup_entity`]'s `EntityKind::parse()` keeps
/// round-tripping on summary rows. `surface` stores the full canonical id
/// as a stable placeholder — at the summary level we have no per-occurrence
/// span to recover, and the id is always unique. The summary's score is
/// reused for each of its entities.
///
/// Callers should prefer the regular [`index_entities_tx`] for leaves,
/// where span/surface are meaningful.
pub(crate) fn index_summary_entity_ids_tx(
    tx: &Transaction<'_>,
    entity_ids: &[String],
    node_id: &str,
    score: f32,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    if entity_ids.is_empty() {
        return Ok(0);
    }
    let mut stmt = tx.prepare(
        "INSERT OR REPLACE INTO mem_tree_entity_index (
            entity_id, node_id, node_kind, entity_kind, surface,
            score, timestamp_ms, tree_id
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?;
    for canonical_id in entity_ids {
        // Canonical ids follow Phase 2's "<kind>:<value>" convention.
        // Without this split, `entity_kind` would hold the full id and
        // `lookup_entity`'s `EntityKind::parse()` would fail at read time,
        // poisoning any mixed leaf/summary lookup.
        let entity_kind = match canonical_id.split_once(':') {
            Some((kind, _)) => kind,
            None => {
                log::warn!(
                    "[memory_tree::score::store] summary entity id missing ':' — \
                     storing as-is: {canonical_id}"
                );
                canonical_id.as_str()
            }
        };
        stmt.execute(params![
            canonical_id,
            node_id,
            "summary",
            entity_kind,
            canonical_id,
            score,
            timestamp_ms,
            tree_id,
        ])?;
    }
    Ok(entity_ids.len())
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

pub fn list_entity_ids_for_node(config: &Config, node_id: &str) -> Result<Vec<String>> {
    with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT entity_id
               FROM mem_tree_entity_index
              WHERE node_id = ?1
              ORDER BY score DESC, timestamp_ms DESC, entity_id ASC",
        )?;
        let rows = stmt
            .query_map(params![node_id], |row| row.get::<_, String>(0))?
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
#[path = "store_tests.rs"]
mod tests;
