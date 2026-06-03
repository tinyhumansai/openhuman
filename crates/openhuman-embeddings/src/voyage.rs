//! Voyage AI embedding provider — delegates to OpenAiEmbedding with Voyage URL.

use async_trait::async_trait;

use crate::openai::OpenAiEmbedding;
use crate::EmbeddingProvider;

pub const VOYAGE_API_BASE: &str = "https://api.voyageai.com";
pub const VOYAGE_DEFAULT_MODEL: &str = "voyage-3-large";
pub const VOYAGE_DEFAULT_DIMS: usize = 1024;

pub struct VoyageEmbedding {
    inner: OpenAiEmbedding,
}

impl VoyageEmbedding {
    pub fn new(api_key: &str, model: &str, dims: usize) -> Self {
        let model = if model.is_empty() { VOYAGE_DEFAULT_MODEL } else { model };
        let dims = if dims == 0 { VOYAGE_DEFAULT_DIMS } else { dims };
        Self { inner: OpenAiEmbedding::new(VOYAGE_API_BASE, api_key, model, dims) }
    }

    pub fn new_with_base_url(api_key: &str, model: &str, dims: usize, base_url: &str) -> Self {
        let model = if model.is_empty() { VOYAGE_DEFAULT_MODEL } else { model };
        let dims = if dims == 0 { VOYAGE_DEFAULT_DIMS } else { dims };
        Self { inner: OpenAiEmbedding::new(base_url, api_key, model, dims) }
    }

    /// Set a custom HTTP client.
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.inner = self.inner.with_client(client);
        self
    }
}

#[async_trait]
impl EmbeddingProvider for VoyageEmbedding {
    fn name(&self) -> &str { "voyage" }
    fn model_id(&self) -> &str { self.inner.model_id() }
    fn dimensions(&self) -> usize { self.inner.dimensions() }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inner.embed(texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_defaults() {
        let p = VoyageEmbedding::new("k", "", 0);
        assert_eq!(p.name(), "voyage");
        assert_eq!(p.model_id(), VOYAGE_DEFAULT_MODEL);
        assert_eq!(p.dimensions(), VOYAGE_DEFAULT_DIMS);
    }

    #[test]
    fn signature() {
        let p = VoyageEmbedding::new("k", "voyage-3-large", 1024);
        assert_eq!(p.signature(), "provider=voyage;model=voyage-3-large;dims=1024");
    }
}
