//! Compatibility wrapper for tinyagents' OpenAI-compatible model.

use async_trait::async_trait;
use tinyagents::harness::embeddings::{EmbeddingModel, OpenAiEmbeddingModel};

use super::EmbeddingProvider;

pub struct OpenAiEmbedding {
    inner: OpenAiEmbeddingModel,
}

impl OpenAiEmbedding {
    pub fn new(base_url: &str, api_key: &str, model: &str, dimensions: usize) -> Self {
        Self {
            inner: OpenAiEmbeddingModel::new(api_key)
                .with_base_url(base_url)
                .with_model(model)
                .with_dimensions(dimensions)
                .with_send_dimensions(false)
                .with_required_api_key(false),
        }
    }

    pub fn with_send_dimensions(mut self, send: bool) -> Self {
        self.inner = self.inner.with_send_dimensions(send);
        self
    }

    pub fn with_required_api_key(mut self, required: bool) -> Self {
        self.inner = self.inner.with_required_api_key(required);
        self
    }

    pub fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    pub fn model(&self) -> &str {
        self.inner.model()
    }

    pub fn embeddings_url(&self) -> String {
        self.inner.embeddings_url()
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiEmbedding {
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }
    fn dimensions(&self) -> usize {
        self.inner.dimensions()
    }
    fn signature(&self) -> String {
        self.inner.signature()
    }
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let texts = texts
            .iter()
            .map(|text| (*text).to_owned())
            .collect::<Vec<_>>();
        self.inner
            .embed(&texts)
            .await
            .map_err(|error| anyhow::anyhow!(error))
    }
}
