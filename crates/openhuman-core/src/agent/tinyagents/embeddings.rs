//! Cost-recording binding over the host's embedding model (issue #4249).
//!
//! The model contract, the signature format and the batch shape all live in
//! `tinyinference-embeddings`; this file only adds OpenHuman's per-call
//! embedding cost recording.

use std::sync::Arc;

use crate::core::runtime::DispatchCtx;
use async_trait::async_trait;
use tinyinference_embeddings::{EmbeddingModel, Result as EmbedResult};

/// Wraps the host-bound [`EmbeddingModel`] (credentials, rate limit and
/// fallback already applied by the factory) and records embedding cost per
/// call. Everything else delegates to the model.
pub(crate) struct ProviderEmbeddingModel {
    model: Arc<dyn EmbeddingModel>,
    dispatch: DispatchCtx,
}

impl ProviderEmbeddingModel {
    /// Builds the cost-recording binding over `model`.
    pub(crate) fn new(model: Arc<dyn EmbeddingModel>, dispatch: DispatchCtx) -> Self {
        tracing::debug!(
            provider = model.name(),
            model_id = model.model_id(),
            dimensions = model.dimensions(),
            signature = %model.signature(),
            "[embeddings] constructing cost-recording embedding binding"
        );
        Self { model, dispatch }
    }
}

#[async_trait]
impl EmbeddingModel for ProviderEmbeddingModel {
    fn is_enabled(&self) -> bool {
        self.model.is_enabled()
    }

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

    async fn embed(&self, texts: &[String]) -> EmbedResult<Vec<Vec<f32>>> {
        let result = self.model.embed(texts).await?;
        // Best-effort cost recording; never fails the embed. Skipped for an
        // empty batch so the request count is not inflated.
        if !result.is_empty() {
            let approx_input_tokens =
                tinyinference_embeddings::estimate_embedding_input_tokens(texts);
            crate::platform::cost::record_embedding_usage(
                &self.dispatch,
                self.model.name(),
                self.model.model_id(),
                approx_input_tokens,
                self.model.dimensions(),
                result.len() as u64,
            );
        }
        Ok(result)
    }
}

#[cfg(test)]
#[path = "embeddings_tests.rs"]
mod tests;
