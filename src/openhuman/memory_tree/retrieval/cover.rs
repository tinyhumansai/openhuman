use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_tree::retrieval::engine::config as engine_config;
use crate::openhuman::memory_tree::retrieval::types::QueryResponse;

const DEFAULT_LIMIT: usize = 200;

pub async fn cover_window(
    config: &Config,
    since_ms: i64,
    until_ms: i64,
    source_id: Option<&str>,
    source_kind: Option<SourceKind>,
    limit: usize,
) -> Result<QueryResponse> {
    let limit = if limit == 0 { DEFAULT_LIMIT } else { limit };
    let scope = current_source_scope();
    if source_id.is_some_and(|id| scope.as_ref().is_some_and(|set| !set.contains(id))) {
        return Ok(QueryResponse::empty());
    }
    log::debug!(
        "[retrieval::cover] tinycortex has_source_id={} source_kind={:?} limit={}",
        source_id.is_some(),
        source_kind.map(|k| k.as_str()),
        limit
    );
    let mut response = tinycortex::memory::retrieval::cover_window_scoped(
        &engine_config(config),
        since_ms,
        until_ms,
        source_id,
        source_kind,
        scope,
        usize::MAX,
    )?;
    let total = response.hits.len();
    response.hits.truncate(limit);
    Ok(QueryResponse::new(response.hits, total))
}
