//! Compatibility wrapper for tinyagents' Cohere model.

use async_trait::async_trait;
use tinyagents::harness::embeddings::{CohereEmbeddingModel, EmbeddingModel};

use super::EmbeddingProvider;

pub struct CohereEmbedding {
    inner: CohereEmbeddingModel,
}

impl CohereEmbedding {
    pub fn new(api_key: &str, model: &str, dimensions: usize) -> Self {
        Self {
            inner: CohereEmbeddingModel::new(api_key)
                .with_model(model)
                .with_dimensions(dimensions),
        }
    }

    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.inner = self.inner.with_base_url(base_url);
        self
    }
}

#[async_trait]
impl EmbeddingProvider for CohereEmbedding {
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
