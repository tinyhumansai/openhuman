//! Product Config adapters over tinycortex score and entity-index persistence.

use std::collections::HashMap;

use anyhow::Result;
use rusqlite::Transaction;

use crate::openhuman::config::Config;

pub use tinycortex::memory::score::store::{EntityHit, ScoreRow};

fn memory_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn upsert_score(config: &Config, row: &ScoreRow) -> Result<()> {
    tinycortex::memory::score::store::upsert_score(&memory_config(config), row)
}

pub(crate) fn upsert_score_tx(tx: &Transaction<'_>, row: &ScoreRow) -> Result<()> {
    tinycortex::memory::score::store::upsert_score_tx(tx, row)
}

pub fn get_score(config: &Config, chunk_id: &str) -> Result<Option<ScoreRow>> {
    tinycortex::memory::score::store::get_score(&memory_config(config), chunk_id)
}

pub fn get_scores_batch(config: &Config, chunk_ids: &[String]) -> Result<HashMap<String, f32>> {
    tinycortex::memory::score::store::get_scores_batch(&memory_config(config), chunk_ids)
}

pub use crate::openhuman::memory_store::entities::{
    clear_entity_index_for_node, count_entity_index, list_entity_ids_for_node, lookup_entity,
};

pub fn index_entity(
    config: &Config,
    entity: &tinycortex::memory::score::resolver::CanonicalEntity,
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<()> {
    let entity = to_store_entity(entity)?;
    crate::openhuman::memory_store::entities::index_entity(
        config,
        &entity,
        node_id,
        node_kind,
        timestamp_ms,
        tree_id,
    )
}

pub fn index_entities(
    config: &Config,
    entities: &[tinycortex::memory::score::resolver::CanonicalEntity],
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    let entities: Vec<tinycortex::memory::store::CanonicalEntity> = entities
        .iter()
        .map(to_store_entity)
        .collect::<Result<_>>()?;
    crate::openhuman::memory_store::entities::index_entities(
        config,
        &entities,
        node_id,
        node_kind,
        timestamp_ms,
        tree_id,
    )
}

pub(crate) fn clear_entity_index_for_node_tx(tx: &Transaction<'_>, node_id: &str) -> Result<usize> {
    tinycortex::memory::score::store::clear_entity_index_for_node_tx(tx, node_id)
}

pub(crate) fn index_summary_entity_ids_tx(
    tx: &Transaction<'_>,
    entity_ids: &[String],
    node_id: &str,
    score: f32,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    let identity = crate::openhuman::memory_store::entities::host_self_identity();
    tinycortex::memory::store::entity_index::index_summary_entity_ids_tx_with_identity(
        tx,
        entity_ids,
        node_id,
        score,
        timestamp_ms,
        tree_id,
        identity.as_ref(),
    )
}

pub(crate) fn index_entities_tx(
    tx: &Transaction<'_>,
    entities: &[tinycortex::memory::score::resolver::CanonicalEntity],
    node_id: &str,
    node_kind: &str,
    timestamp_ms: i64,
    tree_id: Option<&str>,
) -> Result<usize> {
    let identity = crate::openhuman::memory_store::entities::host_self_identity();
    let entities: Vec<tinycortex::memory::store::CanonicalEntity> = entities
        .iter()
        .map(to_store_entity)
        .collect::<Result<_>>()?;
    tinycortex::memory::store::entity_index::index_entities_tx_with_identity(
        tx,
        &entities,
        node_id,
        node_kind,
        timestamp_ms,
        tree_id,
        identity.as_ref(),
    )
}

fn to_store_entity(
    entity: &tinycortex::memory::score::resolver::CanonicalEntity,
) -> Result<tinycortex::memory::store::CanonicalEntity> {
    Ok(tinycortex::memory::store::CanonicalEntity {
        canonical_id: entity.canonical_id.clone(),
        kind: tinycortex::memory::store::EntityKind::parse(entity.kind.as_str())
            .map_err(anyhow::Error::msg)?,
        surface: entity.surface.clone(),
        span_start: entity.span_start,
        span_end: entity.span_end,
        score: entity.score,
    })
}

pub fn count_scores(config: &Config) -> Result<u64> {
    tinycortex::memory::score::store::count_scores(&memory_config(config))
}
