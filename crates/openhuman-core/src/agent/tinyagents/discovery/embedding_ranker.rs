//! Host wiring for the harness `EmbeddingToolRanker`: take the process's
//! `EmbeddingModel` and decide whether the
//! provider can embed at all. The ranker itself (cosine ranking, the in-memory
//! and durable catalogue cache) lives in `tinyagents-harness`
//! (`tool::discover::EmbeddingToolRanker`, `DriverEmbeddingCache`).

use std::sync::Arc;

use tinyagents_harness::tool::discover::{DriverEmbeddingCache, EmbeddingToolRanker};
use tinyinference_embeddings::EmbeddingModel;

/// A ranker over `provider` with an in-memory cache; chain
/// `.with_cache(cache)` to persist it (see [`embedding_tool_ranker_in`]). The
/// cache is keyed by the provider's embedding-space signature, so a model
/// change invalidates it.
pub fn embedding_tool_ranker(provider: Arc<dyn EmbeddingModel>) -> EmbeddingToolRanker {
    let signature = provider.signature();
    EmbeddingToolRanker::new(provider, signature)
}

/// A ranker over `provider` whose catalogue embeddings are cached in the
/// storage backend, in the calling scope (`DriverEmbeddingCache`). When the
/// backend cannot be had the ranker still works, with its in-memory cache
/// only.
pub fn embedding_tool_ranker_in(
    provider: Arc<dyn EmbeddingModel>,
    workspace_dir: &std::path::Path,
) -> EmbeddingToolRanker {
    let ranker = embedding_tool_ranker(provider);
    let opened = crate::storage::local::open("tool_search", workspace_dir).and_then(|o| {
        crate::storage::documents::prepare_once(&o.backend, &o.scope)?;
        o.scoped()
    });
    match opened {
        Ok(scoped) => ranker.with_cache(Arc::new(DriverEmbeddingCache::new(Arc::clone(
            scoped.documents(),
        )))),
        Err(error) => {
            log::warn!("[tool-search] embedding cache unavailable; in-memory only: {error:#}");
            ranker
        }
    }
}

/// Whether `provider` can embed at all. The `none` provider embeds nothing and
/// would rank everything at zero.
pub fn embedding_provider_is_usable(provider: &dyn EmbeddingModel) -> bool {
    provider.dimensions() > 0 && provider.name() != "none"
}

#[cfg(test)]
#[path = "embedding_ranker_tests.rs"]
mod tests;
