//! The opt-in live embedding connectivity and vector-width check.

use async_trait::async_trait;

use crate::doctor::{ComponentId, DoctorCheck, DoctorCtx, DoctorTier, Finding};
use crate::inference::embedding_host::{probe_dimensions_for_inspection, InspectionEmbeddingError};

pub(super) struct EmbeddingsLiveCheck;

#[async_trait]
impl DoctorCheck for EmbeddingsLiveCheck {
    fn id(&self) -> &str {
        "embeddings/live"
    }

    fn category(&self) -> &str {
        "embeddings"
    }

    fn tier(&self) -> DoctorTier {
        DoctorTier::Live
    }

    fn component_gate(&self) -> Option<ComponentId> {
        Some(ComponentId::Memory)
    }

    async fn detect(&self, ctx: &DoctorCtx) -> Vec<Finding> {
        let config = ctx.config();
        if config
            .memory
            .embedding_provider
            .trim()
            .eq_ignore_ascii_case("none")
        {
            return vec![Finding::skipped(
                self.id(),
                "Embedding generation is disabled in the selected configuration.",
                "Select an embedding provider before running the live embedding check.",
            )];
        }

        match probe_dimensions_for_inspection(config).await {
            Ok(dimensions)
                if dimensions.actual > 0 && dimensions.actual == dimensions.configured =>
            {
                vec![Finding::pass(
                    self.id(),
                    "The configured embedding provider returned the expected vector dimensions.",
                )]
            }
            Ok(dimensions) => {
                let message = if dimensions.actual == 0 {
                    format!(
                        "The embedding provider returned an empty vector (0 dimensions); the configuration requests {}.",
                        dimensions.configured
                    )
                } else {
                    format!(
                        "The embedding provider returned {} dimensions; the configuration requests {}.",
                        dimensions.actual, dimensions.configured
                    )
                };
                vec![Finding::error(
                    self.id(),
                    message,
                    "Select a model that matches the configured vector dimensions, or update the configured dimensions.",
                )]
            }
            Err(error) => vec![finding_for_error(self.id(), error)],
        }
    }
}

fn finding_for_error(id: &str, error: InspectionEmbeddingError) -> Finding {
    match error {
        InspectionEmbeddingError::CredentialStoreUnavailable => Finding::error(
            id,
            "The selected embedding credential profile could not be read safely.",
            "Check the selected profile's embedding credential and rerun the live check.",
        ),
        InspectionEmbeddingError::ProviderUnavailable => Finding::error(
            id,
            "The selected embedding provider is unavailable for live inspection.",
            "Configure a supported embedding provider and rerun the live check.",
        ),
        InspectionEmbeddingError::ProbeFailed => Finding::error(
            id,
            "The configured embedding provider could not complete the live probe.",
            "Check the provider endpoint and model, then retry the live check.",
        ),
    }
}

#[cfg(test)]
#[path = "embeddings_live_tests.rs"]
mod tests;
