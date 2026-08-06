//! No-op embedding provider for keyword-only search fallback.

use async_trait::async_trait;
use tinyagents::harness::embeddings::{EmbeddingModel, NoopEmbeddingModel};

use super::EmbeddingProvider;

/// A "no-op" embedding provider used when semantic search is disabled.
/// Returns empty vectors.
pub struct NoopEmbedding;

#[async_trait]
impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &str {
        "none"
    }

    fn model_id(&self) -> &str {
        "none"
    }

    fn dimensions(&self) -> usize {
        0
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let texts = texts
            .iter()
            .map(|text| (*text).to_owned())
            .collect::<Vec<_>>();
        NoopEmbeddingModel
            .embed(&texts)
            .await
            .map_err(|error| anyhow::anyhow!(error))
    }
}
