use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::memory_tree::retrieval::engine::{config as engine_config, EmbedderBridge};
use crate::openhuman::memory_tree::retrieval::types::RetrievalHit;
use crate::openhuman::memory_tree::score::embed::{build_embedder_from_config, InertEmbedder};

pub async fn drill_down(
    config: &Config,
    node_id: &str,
    max_depth: u32,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<RetrievalHit>> {
    log::debug!(
        "[retrieval::drill_down] tinycortex max_depth={} has_query={} limit={:?}",
        max_depth,
        query.is_some(),
        limit
    );
    let embedder = if query.is_none() || max_depth == 0 {
        log::debug!("[retrieval::drill_down] using inert embedder for non-semantic traversal");
        Box::new(InertEmbedder::new())
            as Box<dyn crate::openhuman::memory_tree::score::embed::Embedder>
    } else {
        build_embedder_from_config(config)?
    };
    let bridge = EmbedderBridge(embedder.as_ref());
    let engine_limit = current_source_scope()
        .as_ref()
        .map(|_| None)
        .unwrap_or(limit);
    let mut hits = tinycortex::memory::retrieval::drill_down(
        &engine_config(config),
        node_id,
        max_depth,
        query,
        &bridge,
        engine_limit,
    )
    .await?;
    if let Some(set) = current_source_scope() {
        hits.retain(|hit| set.contains(&hit.tree_scope));
    }
    if let Some(limit) = limit {
        hits.truncate(limit);
    }
    Ok(hits)
}
