//! Compatibility wrapper for tinyagents' Ollama model.

use async_trait::async_trait;
use tinyagents::harness::embeddings::{EmbeddingModel, OllamaEmbeddingModel};

use super::EmbeddingProvider;

pub use tinyagents::harness::embeddings::{
    DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL, DEFAULT_OLLAMA_URL,
};

pub struct OllamaEmbedding {
    inner: OllamaEmbeddingModel,
}

impl OllamaEmbedding {
    pub fn try_new(base_url: &str, model: &str, dimensions: usize) -> anyhow::Result<Self> {
        Ok(Self {
            inner: OllamaEmbeddingModel::try_new(base_url, model, dimensions)?,
        })
    }

    pub fn new(base_url: &str, model: &str, dimensions: usize) -> Self {
        Self::try_new(base_url, model, dimensions).expect("invalid Ollama embedding configuration")
    }

    pub fn base_url(&self) -> &str {
        self.inner.base_url()
    }

    pub fn model(&self) -> &str {
        self.inner.model()
    }
}

impl Default for OllamaEmbedding {
    fn default() -> Self {
        Self {
            inner: OllamaEmbeddingModel::default(),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for OllamaEmbedding {
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
