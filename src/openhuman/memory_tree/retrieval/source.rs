use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_tree::retrieval::engine::{config as engine_config, EmbedderBridge};
use crate::openhuman::memory_tree::retrieval::types::QueryResponse;
use crate::openhuman::memory_tree::score::embed::build_embedder_from_config;

const DEFAULT_LIMIT: usize = 10;

pub async fn query_source(
    config: &Config,
    source_id: Option<&str>,
    source_kind: Option<SourceKind>,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<QueryResponse> {
    let limit = if limit == 0 { DEFAULT_LIMIT } else { limit };
    let scope = current_source_scope();
    if source_id.is_some_and(|id| scope.as_ref().is_some_and(|set| !set.contains(id))) {
        log::debug!("[retrieval::source] explicit source excluded by active scope");
        return Ok(QueryResponse::empty());
    }

    log::debug!(
        "[retrieval::source] tinycortex query has_source_id={} source_kind={:?} window_days={:?} has_query={} limit={}",
        source_id.is_some(), source_kind.map(|k| k.as_str()), time_window_days, query.is_some(), limit
    );
    let semantic_query = query.filter(|value| !value.trim().is_empty());
    let mut response = if let Some(query) = semantic_query {
        let embedder = build_embedder_from_config(config)?;
        let bridge = EmbedderBridge(embedder.as_ref());
        tinycortex::memory::retrieval::query_source(
            &engine_config(config),
            source_id,
            source_kind,
            time_window_days,
            Some(query),
            &bridge,
            usize::MAX,
        )
        .await?
    } else {
        tinycortex::memory::retrieval::query_source(
            &engine_config(config),
            source_id,
            source_kind,
            time_window_days,
            None,
            &tinycortex::memory::score::embed::InertEmbedder::new(),
            usize::MAX,
        )
        .await?
    };
    if let Some(set) = scope {
        response.hits.retain(|hit| set.contains(&hit.tree_scope));
    }
    let total = response.hits.len();
    response.hits.truncate(limit);
    Ok(QueryResponse::new(response.hits, total))
}
