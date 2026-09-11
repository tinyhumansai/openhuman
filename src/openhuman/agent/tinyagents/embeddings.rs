//! Adapter bridging OpenHuman's [`EmbeddingProvider`] onto the `tinyagents`
//! crate's [`EmbeddingModel`] trait (issue #4249, workstream 09-embeddings).
//!
//! OpenHuman owns the concrete embedding providers (voyage / openai / cohere /
//! ollama / cloud / noop) and the `embeddings/factory.rs` construction policy
//! (rate-limit + retry). This adapter is a **thin seam**: it wraps an
//! `Arc<dyn EmbeddingProvider>` and re-exposes it as the crate's provider-neutral
//! [`EmbeddingModel`] so the harness's retrieval surface
//! ([`Retriever`](tinyinference::embeddings::Retriever) /
//! [`VectorStore`](tinyinference::embeddings::VectorStore)) can drive
//! OpenHuman embeddings without cloning provider logic.
//!
//! The only real work here is bridging the batch signature: the crate trait
//! takes `&[String]` while OpenHuman's `EmbeddingProvider::embed` takes
//! `&[&str]`, and the crate uses its own `TinyAgentsError` while OpenHuman uses
//! `anyhow::Error`. Both are mapped without touching the underlying providers.
//!
//! Wired into the recall/retrieval path in step 09.2; this step just lands the
//! adapter + test so it compiles and is available. The `pub(crate)` re-export
//! from `mod.rs` keeps it on the crate surface so it is not dead code.

use std::sync::Arc;

use async_trait::async_trait;
use tinyinference::embeddings::EmbeddingModel as TaEmbeddingModel;
use tinyinference::{Error as TiError, Result as TaResult};

use crate::openhuman::inference::embeddings::EmbeddingProvider;

/// Wraps an OpenHuman [`EmbeddingProvider`] as a `tinyagents`
/// [`EmbeddingModel`](TaEmbeddingModel).
///
/// Holds the provider behind an `Arc` (matching how providers are shared
/// elsewhere in the codebase), so the adapter is cheap to clone and share
/// across async task boundaries behind an `Arc<dyn EmbeddingModel>`.
pub(crate) struct ProviderEmbeddingModel {
    /// The underlying OpenHuman embedding provider (voyage/openai/cohere/ollama/
    /// cloud/noop) with its factory-configured rate-limit + retry policy intact.
    provider: Arc<dyn EmbeddingProvider>,
}

impl ProviderEmbeddingModel {
    /// Builds an adapter over the given OpenHuman embedding provider.
    pub(crate) fn new(provider: Arc<dyn EmbeddingProvider>) -> Self {
        tracing::debug!(
            provider = provider.name(),
            model_id = provider.model_id(),
            dimensions = provider.dimensions(),
            signature = %provider.signature(),
            "[embeddings] constructing tinyagents EmbeddingModel adapter over EmbeddingProvider"
        );
        Self { provider }
    }

    /// Returns the wrapped provider's stable embedding-space signature
    /// (`provider=…;model=…;dims=…`). Preserved so a downstream vector store
    /// keyed on the signature stays byte-identical to one keyed on the raw
    /// provider (#1574 fidelity).
    #[allow(dead_code)] // Signature routing is wired into the recall facade in 09.2.
    pub(crate) fn signature(&self) -> String {
        self.provider.signature()
    }
}

#[async_trait]
impl TaEmbeddingModel for ProviderEmbeddingModel {
    fn name(&self) -> &str {
        self.provider.name()
    }

    fn model_id(&self) -> &str {
        self.provider.model_id()
    }

    async fn embed(&self, texts: &[String]) -> TaResult<Vec<Vec<f32>>> {
        tracing::debug!(
            provider = self.provider.name(),
            batch = texts.len(),
            "[embeddings] adapter embed: entry"
        );
        // Bridge the signature difference: the crate trait hands us owned
        // `String`s; OpenHuman's `EmbeddingProvider::embed` takes borrowed
        // `&str`. Borrow each without allocating new strings.
        let borrowed: Vec<&str> = texts.iter().map(String::as_str).collect();
        let result = self.provider.embed(&borrowed).await.map_err(|e| {
            tracing::warn!(
                provider = self.provider.name(),
                error = %e,
                "[embeddings] adapter embed: provider error"
            );
            // OpenHuman providers surface `anyhow::Error`; the crate expects its
            // own error type. Carry the full chain into the crate's embedding
            // error variant so nothing is lost.
            TiError::Embedding(format!("{e:#}"))
        })?;
        tracing::debug!(
            provider = self.provider.name(),
            vectors = result.len(),
            "[embeddings] adapter embed: exit"
        );
        // Best-effort embedding cost recording (06-cost step 4 / 09-embeddings
        // step 4). Records provider, model, approximate input tokens, dims, and
        // vector count as a CostRecord priced via the unified catalog. Never
        // fail an embed because cost recording failed — `record_embedding_usage`
        // swallows its own errors; we only skip the accounting call for an empty
        // batch (a non-event) so the request count isn't inflated.
        if !result.is_empty() {
            // Rough token estimate from character count (~4 chars/token). The
            // exact value only affects the catalog price when an embedding rate
            // exists; embedding models are usually uncatalogued, in which case
            // the recorded cost is zero regardless.
            let total_chars: usize = texts.iter().map(|t| t.chars().count()).sum();
            let approx_input_tokens = (total_chars as u64).div_ceil(4);
            crate::openhuman::platform::cost::record_embedding_usage(
                self.provider.name(),
                self.provider.model_id(),
                approx_input_tokens,
                self.provider.dimensions(),
                result.len() as u64,
            );
        }
        Ok(result)
    }

    fn dimensions(&self) -> usize {
        self.provider.dimensions()
    }
}

#[cfg(test)]
#[path = "embeddings_tests.rs"]
mod tests;
