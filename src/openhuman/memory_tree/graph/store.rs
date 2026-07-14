//! `Config` adapters for tinycortex-owned persisted graph edges.

use anyhow::Result;
use rusqlite::Transaction;

use crate::openhuman::config::Config;

pub use tinycortex::memory::graph::pairs_from_entities;

fn engine_config(config: &Config) -> tinycortex::memory::MemoryConfig {
    crate::openhuman::tinycortex::memory_config_from(config, config.workspace_dir.clone())
}

pub fn upsert_edges_tx(
    transaction: &Transaction<'_>,
    pairs: &[(String, String)],
    timestamp_ms: i64,
) -> Result<usize> {
    tinycortex::memory::graph::upsert_edges_tx(transaction, pairs, timestamp_ms)
}

pub fn upsert_edges(
    config: &Config,
    pairs: &[(String, String)],
    timestamp_ms: i64,
) -> Result<usize> {
    tinycortex::memory::graph::upsert_edges(&engine_config(config), pairs, timestamp_ms)
}

pub fn neighbors(config: &Config, entity_id: &str) -> Result<Vec<(String, i64)>> {
    tinycortex::memory::graph::edge_neighbors(&engine_config(config), entity_id)
}

pub fn clear_edges_for_entities_tx(
    transaction: &Transaction<'_>,
    entity_ids: &[String],
) -> Result<usize> {
    tinycortex::memory::graph::clear_edges_for_entities_tx(transaction, entity_ids)
}

pub fn count_edges(config: &Config) -> Result<u64> {
    tinycortex::memory::graph::count_edges(&engine_config(config))
}
