use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::source_scope::chunk_source_allowed_in;
use crate::openhuman::memory::source_scope::current_source_scope;
use crate::openhuman::memory_store::chunks::store::get_chunks_batch;
use crate::openhuman::memory_tree::retrieval::engine::config as engine_config;
use crate::openhuman::memory_tree::retrieval::types::RetrievalHit;

pub use tinycortex::memory::retrieval::MAX_BATCH;

pub async fn fetch_leaves(config: &Config, chunk_ids: &[String]) -> Result<Vec<RetrievalHit>> {
    log::debug!(
        "[retrieval::fetch] tinycortex requested={}",
        chunk_ids.len()
    );
    let permitted_ids = if let Some(set) = current_source_scope() {
        let chunks = get_chunks_batch(config, chunk_ids)?;
        chunk_ids
            .iter()
            .filter(|id| {
                chunks.get(*id).is_some_and(|chunk| {
                    chunk_source_allowed_in(&set, &chunk.metadata.tags, &chunk.metadata.source_id)
                })
            })
            .cloned()
            .collect::<Vec<_>>()
    } else {
        chunk_ids.to_vec()
    };
    tinycortex::memory::retrieval::fetch_leaves(&engine_config(config), &permitted_ids)
}
