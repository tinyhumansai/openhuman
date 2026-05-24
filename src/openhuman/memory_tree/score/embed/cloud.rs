//! Cloud (Voyage-backed) embedder for the memory tree.
//!
//! Adapts the OpenHuman backend's `POST /openai/v1/embeddings` surface
//! (Voyage `voyage-3.5`, 1024 dims) to the memory_tree [`Embedder`] trait
//! so Phase 4 ingest / bucket-seal can vectorize chunks without a local
//! Ollama install.
//!
//! The 1024-dim output matches existing on-disk blobs (which were
//! produced by `bge-m3`, also 1024-dim), so this is a drop-in replacement
//! for the Ollama path — no migration of `mem_tree_chunks.embedding`
//! required.
//!
//! Auth: the cloud embedder resolves the session JWT per call via
//! [`OpenHumanCloudEmbedding`], so a session refresh between batches is
//! picked up transparently. When the user is unauthenticated the first
//! `embed()` returns an error; ingest treats that the same as any other
//! embedder failure (don't persist the row, let job retry).

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{Embedder, EMBEDDING_DIM};
use crate::openhuman::config::Config;
use crate::openhuman::embeddings::cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
use crate::openhuman::embeddings::EmbeddingProvider;

/// Cloud-backed memory_tree embedder.
///
/// Wraps [`OpenHumanCloudEmbedding`] (which speaks the OpenAI-compatible
/// `/openai/v1/embeddings` shape backed by Voyage on the OpenHuman
/// backend) and adapts it to the memory_tree [`Embedder`] trait.
pub struct CloudEmbedder {
    inner: OpenHumanCloudEmbedding,
}

impl CloudEmbedder {
    /// Build a cloud embedder using the same backend resolution as the
    /// main embeddings path: `api_url` falls back to
    /// [`effective_api_url`](crate::api::config::effective_api_url) and
    /// the workspace dir comes from `config.workspace_dir` so the auth
    /// service finds the user's session JWT.
    pub fn new(config: &Config) -> Self {
        let openhuman_dir = config.config_path.parent().map(std::path::PathBuf::from);
        Self {
            inner: OpenHumanCloudEmbedding::new(
                None,
                openhuman_dir,
                config.secrets.encrypt,
                DEFAULT_CLOUD_EMBEDDING_MODEL,
                DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
            ),
        }
    }
}

#[async_trait]
impl Embedder for CloudEmbedder {
    fn name(&self) -> &'static str {
        "cloud"
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let v = self
            .inner
            .embed_one(text)
            .await
            .context("cloud embeddings failed")?;
        if v.len() != EMBEDDING_DIM {
            anyhow::bail!(
                "cloud embedder returned {} dims, expected {}",
                v.len(),
                EMBEDDING_DIM
            );
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.config_path = tmp.path().join("config.toml");
        (tmp, cfg)
    }

    #[test]
    fn name_is_cloud() {
        let (_tmp, cfg) = test_config();
        let e = CloudEmbedder::new(&cfg);
        assert_eq!(e.name(), "cloud");
    }
}
