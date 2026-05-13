//! Cloud embedding provider — routes through the OpenHuman backend's
//! `POST /openai/v1/embeddings` surface (Voyage-backed) using the same
//! session JWT that the `OpenHumanBackendProvider` chat path uses.
//!
//! This is the default embedder for a fresh install. The local Ollama path
//! stays available, but the user has to explicitly opt in (either by setting
//! `memory.embedding_provider = "ollama"` in `config.toml`, or by enabling
//! the local-AI runtime with `local_ai.usage.embeddings = true`).
//!
//! The JWT and API URL are resolved per call so a session refresh between
//! embed batches is picked up transparently — matching
//! [`crate::openhuman::providers::openhuman_backend::OpenHumanBackendProvider`].

use std::path::PathBuf;

use async_trait::async_trait;

use super::openai::OpenAiEmbedding;
use super::EmbeddingProvider;
use crate::api::config::effective_api_url;
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER};

/// Default cloud embedding model — backed by `voyage-3.5` (1024 dims) on the
/// OpenHuman backend. See `tinyhumansai/backend#746`.
pub const DEFAULT_CLOUD_EMBEDDING_MODEL: &str = "embedding-v1";

/// Default output dimensionality for [`DEFAULT_CLOUD_EMBEDDING_MODEL`].
pub const DEFAULT_CLOUD_EMBEDDING_DIMENSIONS: usize = 1024;

/// OpenHuman-backend-backed embedding provider.
pub struct OpenHumanCloudEmbedding {
    api_url: Option<String>,
    openhuman_dir: Option<PathBuf>,
    secrets_encrypt: bool,
    model: String,
    dims: usize,
}

impl OpenHumanCloudEmbedding {
    /// Construct a cloud embedder. `api_url` and `openhuman_dir` are looked up
    /// per request; pass `None` to fall back to the runtime defaults
    /// ([`effective_api_url`] / `~/.openhuman`).
    pub fn new(
        api_url: Option<String>,
        openhuman_dir: Option<PathBuf>,
        secrets_encrypt: bool,
        model: impl Into<String>,
        dims: usize,
    ) -> Self {
        Self {
            api_url: api_url
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            openhuman_dir,
            secrets_encrypt,
            model: model.into(),
            dims,
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.openhuman_dir.clone().unwrap_or_else(|| {
            directories::UserDirs::new()
                .map(|d| d.home_dir().join(".openhuman"))
                .unwrap_or_else(|| PathBuf::from(".openhuman"))
        })
    }

    fn resolve_bearer(&self) -> anyhow::Result<String> {
        let auth = AuthService::new(&self.state_dir(), self.secrets_encrypt);
        if let Some(t) = auth
            .get_provider_bearer_token(APP_SESSION_PROVIDER, None)?
            .filter(|s| !s.trim().is_empty())
        {
            return Ok(t);
        }
        anyhow::bail!(
            "No backend session for cloud embeddings: log in to OpenHuman, or set \
             memory.embedding_provider to \"ollama\" / \"none\" in config.toml"
        )
    }

    fn base_url(&self) -> String {
        let u = effective_api_url(&self.api_url);
        format!("{}/openai/v1", u.trim_end_matches('/'))
    }
}

#[async_trait]
impl EmbeddingProvider for OpenHumanCloudEmbedding {
    fn name(&self) -> &str {
        "cloud"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let token = self.resolve_bearer()?;
        let inner = OpenAiEmbedding::new(&self.base_url(), &token, &self.model, self.dims);
        inner.embed(texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_and_dimensions() {
        let p = OpenHumanCloudEmbedding::new(
            None,
            None,
            true,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        );
        assert_eq!(p.name(), "cloud");
        assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
    }

    #[test]
    fn base_url_appends_openai_v1() {
        let p = OpenHumanCloudEmbedding::new(
            Some("https://api.openhuman.example/".into()),
            None,
            true,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        );
        assert_eq!(p.base_url(), "https://api.openhuman.example/openai/v1");
    }

    #[tokio::test]
    async fn embed_empty_returns_empty_without_auth() {
        // Empty input should short-circuit *before* hitting the AuthService —
        // otherwise the no-op path would spuriously fail in unauthenticated
        // contexts (e.g. ingestion of an empty chunk batch).
        let p = OpenHumanCloudEmbedding::new(
            None,
            None,
            false,
            DEFAULT_CLOUD_EMBEDDING_MODEL,
            DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        );
        assert!(p.embed(&[]).await.unwrap().is_empty());
    }
}
