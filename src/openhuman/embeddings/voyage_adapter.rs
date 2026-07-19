//! Compatibility wrapper for tinyagents' Voyage model.

use async_trait::async_trait;
use tinyagents::harness::embeddings::{
    EmbeddingModel, VoyageEmbeddingModel, VOYAGE_API_BASE, VOYAGE_DEFAULT_DIMENSIONS,
    VOYAGE_DEFAULT_MODEL,
};

use super::EmbeddingProvider;

pub struct VoyageEmbedding {
    inner: VoyageEmbeddingModel,
}

impl VoyageEmbedding {
    pub fn new(api_key: &str, model: &str, dimensions: usize) -> Self {
        Self::new_with_base_url(api_key, model, dimensions, VOYAGE_API_BASE)
    }

    pub fn new_with_base_url(
        api_key: &str,
        model: &str,
        dimensions: usize,
        base_url: &str,
    ) -> Self {
        Self {
            inner: VoyageEmbeddingModel::with_options(
                api_key,
                if model.is_empty() {
                    VOYAGE_DEFAULT_MODEL
                } else {
                    model
                },
                if dimensions == 0 {
                    VOYAGE_DEFAULT_DIMENSIONS
                } else {
                    dimensions
                },
                base_url,
            ),
        }
    }
}

#[async_trait]
impl EmbeddingProvider for VoyageEmbedding {
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
