//! Interface for embedding providers that convert text into numerical vectors.

use async_trait::async_trait;

/// Interface for embedding providers that convert text into numerical vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns the name of the provider (e.g., "ollama", "openai").
    fn name(&self) -> &str;

    /// Returns the stable model identifier used to generate embeddings.
    fn model_id(&self) -> &str;

    /// Returns the number of dimensions in the generated embeddings.
    fn dimensions(&self) -> usize;

    /// Returns a stable signature for the embedding space.
    ///
    /// Changing any component means existing vectors may no longer be
    /// comparable with newly-generated vectors and should be stored / queried
    /// separately by follow-up storage migrations.
    fn signature(&self) -> String {
        format!(
            "provider={};model={};dims={}",
            self.name(),
            self.model_id(),
            self.dimensions()
        )
    }

    /// Generates embeddings for a batch of strings.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Generates an embedding for a single string.
    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }
}
