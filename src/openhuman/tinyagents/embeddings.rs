//! Adapter bridging OpenHuman's [`EmbeddingProvider`] onto the `tinyagents`
//! crate's [`EmbeddingModel`] trait (issue #4249, workstream 09-embeddings).
//!
//! OpenHuman owns the concrete embedding providers (voyage / openai / cohere /
//! ollama / cloud / noop) and the `embeddings/factory.rs` construction policy
//! (rate-limit + retry). This adapter is a **thin seam**: it wraps an
//! `Arc<dyn EmbeddingProvider>` and re-exposes it as the crate's provider-neutral
//! [`EmbeddingModel`] so the harness's retrieval surface
//! ([`Retriever`](tinyagents::harness::embeddings::Retriever) /
//! [`VectorStore`](tinyagents::harness::embeddings::VectorStore)) can drive
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
use tinyagents::harness::embeddings::EmbeddingModel as TaEmbeddingModel;
use tinyagents::{Result as TaResult, TinyAgentsError};

use crate::openhuman::embeddings::EmbeddingProvider;

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
            TinyAgentsError::Embedding(format!("{e:#}"))
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
            crate::openhuman::cost::record_embedding_usage(
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
mod tests {
    use super::*;

    /// Deterministic stub provider: emits one fixed-length vector per input,
    /// each component seeded from the input length so distinct texts map to
    /// distinct vectors and the round-trip / dimension assertions are stable.
    struct StubProvider {
        dims: usize,
    }

    #[async_trait]
    impl EmbeddingProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn model_id(&self) -> &str {
            "stub-model"
        }

        fn dimensions(&self) -> usize {
            self.dims
        }

        async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(texts
                .iter()
                .map(|t| vec![t.len() as f32; self.dims])
                .collect())
        }
    }

    #[tokio::test]
    async fn embeddings_adapter_round_trips_and_reports_dimensions() {
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(StubProvider { dims: 4 });
        let adapter = ProviderEmbeddingModel::new(provider);

        // dimensions() is forwarded from the underlying provider.
        assert_eq!(TaEmbeddingModel::name(&adapter), "stub");
        assert_eq!(TaEmbeddingModel::model_id(&adapter), "stub-model");
        assert_eq!(TaEmbeddingModel::dimensions(&adapter), 4);

        // embed() bridges `&[String]` -> `&[&str]` and returns one vector per
        // input, in order, each of the reported dimensionality.
        let inputs = vec!["ab".to_string(), "abcd".to_string()];
        let vectors = adapter.embed(&inputs).await.expect("embed should succeed");
        assert_eq!(vectors.len(), 2);
        assert!(vectors.iter().all(|v| v.len() == 4));
        // Distinct inputs -> distinct vectors (seeded from text length).
        assert_eq!(vectors[0], vec![2.0; 4]);
        assert_eq!(vectors[1], vec![4.0; 4]);
    }

    #[tokio::test]
    async fn embeddings_adapter_preserves_signature() {
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(StubProvider { dims: 8 });
        let expected = provider.signature();
        let adapter = ProviderEmbeddingModel::new(provider);
        assert_eq!(adapter.signature(), expected);
        assert_eq!(adapter.signature(), "provider=stub;model=stub-model;dims=8");
    }

    #[tokio::test]
    async fn embeddings_adapter_empty_batch_is_empty() {
        let provider: Arc<dyn EmbeddingProvider> = Arc::new(StubProvider { dims: 3 });
        let adapter = ProviderEmbeddingModel::new(provider);
        let vectors = adapter.embed(&[]).await.expect("empty embed ok");
        assert!(vectors.is_empty());
    }
}
