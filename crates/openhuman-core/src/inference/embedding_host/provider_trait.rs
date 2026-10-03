//! The embedding provider seam and its TinyInference adapter.
//!
//! [`EmbeddingProvider`] and [`format_embedding_signature`] are defined in
//! [`super::embedding_trait`] (they used to live in the v1 memory contract,
//! which no longer exists). [`TinyInferenceEmbeddingProvider`] adapts a
//! TinyInference `EmbeddingModel` onto that trait; every construction site
//! (`factory.rs`, `cloud_adapter.rs`) builds a model and wraps it here.
//!
//! The signature format is load-bearing: [`format_embedding_signature`] is the
//! single source of truth, so a vector written before and after a refactor
//! lands in the same embedding space.

use async_trait::async_trait;
use tinyinference_embeddings::EmbeddingModel;

pub use super::embedding_trait::{format_embedding_signature, EmbeddingProvider};

/// Adapts the canonical TinyInference model to the memory-host contract.
pub struct TinyInferenceEmbeddingProvider {
    model: Box<dyn EmbeddingModel>,
}

impl TinyInferenceEmbeddingProvider {
    pub fn new(model: impl EmbeddingModel + 'static) -> Self {
        Self {
            model: Box::new(model),
        }
    }

    pub fn boxed(model: impl EmbeddingModel + 'static) -> Box<dyn EmbeddingProvider> {
        Box::new(Self::new(model))
    }

    pub fn from_boxed(model: Box<dyn EmbeddingModel>) -> Self {
        Self { model }
    }
}

#[async_trait]
impl EmbeddingProvider for TinyInferenceEmbeddingProvider {
    fn name(&self) -> &str {
        self.model.name()
    }

    fn model_id(&self) -> &str {
        self.model.model_id()
    }

    fn dimensions(&self) -> usize {
        self.model.dimensions()
    }

    fn signature(&self) -> String {
        self.model.signature()
    }

    /// The one shape difference between the two traits: the contract takes
    /// `&[&str]` and `EmbeddingModel` takes `&[String]`, so the batch is owned
    /// here. Kept verbatim from the engine-side adapter this replaces — a
    /// "cheaper" variant that embedded one text at a time would turn one
    /// provider request into N.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let owned = texts
            .iter()
            .map(|text| (*text).to_owned())
            .collect::<Vec<_>>();
        self.model
            .embed(&owned)
            .await
            .map_err(|error| anyhow::anyhow!(error))
    }
}
