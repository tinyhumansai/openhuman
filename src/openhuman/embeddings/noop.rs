//! No-op embedding provider for keyword-only search fallback.

use async_trait::async_trait;

use super::EmbeddingProvider;

/// A "no-op" embedding provider used when semantic search is disabled.
/// Returns empty vectors.
pub struct NoopEmbedding;

#[async_trait]
impl EmbeddingProvider for NoopEmbedding {
    fn name(&self) -> &str {
        "none"
    }

    fn dimensions(&self) -> usize {
        0
    }

    async fn embed(&self, _texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(Vec::new())
    }
}
