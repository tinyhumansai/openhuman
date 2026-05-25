//! Voyage AI embedding provider — direct API access with user's own key.
//!
//! Voyage's `/v1/embeddings` endpoint speaks a superset of the OpenAI
//! embeddings contract (same request/response shape, plus extra fields
//! like `input_type` and `truncation`). We delegate to `OpenAiEmbedding`
//! for the HTTP plumbing and just set the correct base URL + auth.

use async_trait::async_trait;

use super::openai::OpenAiEmbedding;
use super::EmbeddingProvider;

pub const VOYAGE_API_BASE: &str = "https://api.voyageai.com";
pub const VOYAGE_DEFAULT_MODEL: &str = "voyage-3-large";
pub const VOYAGE_DEFAULT_DIMS: usize = 1024;

pub struct VoyageEmbedding {
    inner: OpenAiEmbedding,
}

impl VoyageEmbedding {
    pub fn new(api_key: &str, model: &str, dims: usize) -> Self {
        let model = if model.is_empty() {
            VOYAGE_DEFAULT_MODEL
        } else {
            model
        };
        let dims = if dims == 0 { VOYAGE_DEFAULT_DIMS } else { dims };

        Self {
            inner: OpenAiEmbedding::new(VOYAGE_API_BASE, api_key, model, dims),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for VoyageEmbedding {
    fn name(&self) -> &str {
        "voyage"
    }

    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        self.inner.embed(texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_defaults() {
        let p = VoyageEmbedding::new("test-key", "", 0);
        assert_eq!(p.name(), "voyage");
        assert_eq!(p.model_id(), VOYAGE_DEFAULT_MODEL);
        assert_eq!(p.dimensions(), VOYAGE_DEFAULT_DIMS);
    }

    #[test]
    fn custom_model_and_dims() {
        let p = VoyageEmbedding::new("test-key", "voyage-code-3", 1024);
        assert_eq!(p.model_id(), "voyage-code-3");
        assert_eq!(p.dimensions(), 1024);
    }

    #[test]
    fn signature_format() {
        let p = VoyageEmbedding::new("k", "voyage-3-large", 1024);
        assert_eq!(
            p.signature(),
            "provider=voyage;model=voyage-3-large;dims=1024"
        );
    }
}
