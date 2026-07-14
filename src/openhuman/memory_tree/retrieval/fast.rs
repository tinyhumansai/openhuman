//! Product adapters for tinycortex-owned deterministic fast retrieval.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::memory_tree::nlp;
use crate::openhuman::memory_tree::retrieval::engine::{config as engine_config, EmbedderBridge};
use crate::openhuman::memory_tree::retrieval::types::QueryResponse;
use crate::openhuman::memory_tree::score::embed::build_embedder_from_config;

pub use tinycortex::memory::retrieval::FastRetrieveOptions;

pub async fn fast_retrieve(
    config: &Config,
    query: &str,
    options: FastRetrieveOptions,
) -> Result<QueryResponse> {
    let query_entities = nlp::extract_query_entities(config, query).await;
    let entity_ids: Vec<_> = query_entities
        .into_iter()
        .map(|entity| entity.canonical_id)
        .collect();
    log::debug!(
        "[retrieval::fast] tinycortex query_len={} entities={} limit={} hops={}",
        query.len(),
        entity_ids.len(),
        options.limit,
        options.max_hops
    );
    let embedder = build_embedder_from_config(config)?;
    tinycortex::memory::retrieval::fast_retrieve(
        &engine_config(config),
        query,
        &entity_ids,
        &EmbedderBridge(embedder.as_ref()),
        current_source_scope().as_ref(),
        options,
    )
    .await
}
