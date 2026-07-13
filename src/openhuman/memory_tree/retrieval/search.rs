use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_tree::retrieval::engine::config as engine_config;
use crate::openhuman::memory_tree::retrieval::types::EntityMatch;
use crate::openhuman::memory_tree::score::extract::EntityKind;

pub async fn search_entities(
    config: &Config,
    query: &str,
    kinds: Option<Vec<EntityKind>>,
    limit: usize,
) -> Result<Vec<EntityMatch>> {
    log::debug!(
        "[retrieval::search] tinycortex query_len={} kinds={} limit={}",
        query.len(),
        kinds.as_ref().map_or(0, Vec::len),
        limit
    );
    tinycortex::memory::retrieval::search_entities(
        &engine_config(config),
        query,
        kinds.as_deref(),
        limit,
    )
}
