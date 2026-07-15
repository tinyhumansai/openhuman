//! OpenHuman credential adapter for tinyagents cloud embeddings.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents::harness::embeddings::{
    BearerResolver, CloudEmbeddingModel, DEFAULT_CLOUD_DIMENSIONS, DEFAULT_CLOUD_MODEL,
};

use super::{EmbeddingProvider, TinyAgentsEmbeddingProvider};
use crate::api::config::effective_api_url;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};

pub const DEFAULT_CLOUD_EMBEDDING_MODEL: &str = DEFAULT_CLOUD_MODEL;
pub const DEFAULT_CLOUD_EMBEDDING_DIMENSIONS: usize = DEFAULT_CLOUD_DIMENSIONS;

/// Host-owned credential resolution around the crate-owned cloud transport.
pub struct OpenHumanCloudEmbedding {
    inner: TinyAgentsEmbeddingProvider,
}

impl OpenHumanCloudEmbedding {
    pub fn new(
        api_url: Option<String>,
        openhuman_dir: Option<PathBuf>,
        secrets_encrypt: bool,
        model: impl Into<String>,
        dimensions: usize,
    ) -> Self {
        let state_dir = openhuman_dir.unwrap_or_else(default_state_dir);
        let bearer: BearerResolver = Arc::new(move || {
            let auth = AuthService::new(&state_dir, secrets_encrypt);
            auth.get_provider_bearer_token(APP_SESSION_PROVIDER, None)
                .map_err(|error| tinyagents::TinyAgentsError::Embedding(error.to_string()))?
                .filter(|token| !token.trim().is_empty())
                .ok_or_else(|| {
                    tinyagents::TinyAgentsError::Validation(
                        "No backend session for cloud embeddings: log in to OpenHuman".into(),
                    )
                })
        });
        let base_url = format!(
            "{}/openai/v1",
            effective_api_url(&api_url).trim_end_matches('/')
        );
        Self {
            inner: TinyAgentsEmbeddingProvider::new(CloudEmbeddingModel::new(
                base_url, model, dimensions, bearer,
            )),
        }
    }
}

fn default_state_dir() -> PathBuf {
    if let Some(workspace) = std::env::var_os("OPENHUMAN_WORKSPACE")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
    {
        return workspace;
    }
    directories::UserDirs::new()
        .map(|dirs| dirs.home_dir().join(".openhuman"))
        .unwrap_or_else(|| PathBuf::from(".openhuman"))
}

#[async_trait]
impl EmbeddingProvider for OpenHumanCloudEmbedding {
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
        // Egress spine (privacy epic S2, #4436): the input texts leave the
        // device for the cloud embedding backend — disclose before the request.
        let egress = crate::openhuman::security::egress::EgressDescriptor::embedding(
            "cloud",
            self.inner.model_id(),
        );
        // Local-only enforcement (privacy epic S7, #4441): refuse cloud
        // embedding under LocalOnly before disclosing or sending the texts.
        crate::openhuman::security::egress::enforce_egress(&egress)?;
        crate::openhuman::security::egress::emit_external_transfer(egress);
        self.inner.embed(texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Privacy epic S7 (#4441): under LocalOnly, `embed` refuses before touching
    /// the inner cloud transport (so no network / no auth is needed to observe
    /// the block). Uses the thread-scoped privacy override so it never mutates
    /// the process-global policy that sibling tests read.
    #[tokio::test]
    async fn embed_blocked_under_local_only() {
        let _mode = crate::openhuman::security::live_policy::test_privacy_scope(
            crate::openhuman::config::PrivacyMode::LocalOnly,
        );
        let provider = OpenHumanCloudEmbedding::new(
            Some("http://127.0.0.1:0".into()),
            Some(std::env::temp_dir().join("openhuman_embeddings_localonly_state")),
            false,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        );

        let err = provider.embed(&["hello"]).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("Local-only privacy mode is active"),
            "unexpected error: {err}"
        );
    }
}
