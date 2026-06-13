//! Memory-tree [`Embedder`] backed by a user-configured OpenAI-compatible
//! embeddings provider (#002 FR-015).
//!
//! ## Why this exists
//!
//! The memory-tree embedder factory historically resolved only: explicit
//! Ollama override → `ollama:` workload prefix → managed `CloudEmbedder`
//! (backend→Voyage) → skip. So a user who configured **OpenAI** (or any
//! custom OpenAI-compatible endpoint) in Settings → AI → Embeddings was
//! silently ignored: their `embeddings_provider = "openai"` matched no branch
//! and fell through to the managed backend, which then hit "managed budget"
//! while the user's own key sat unused. This adapter closes that gap.
//!
//! ## How
//!
//! It wraps the unified [`EmbeddingProvider`] built by
//! [`create_embedding_provider_with_credentials`] (the same construction the
//! Settings "Test connection" + main embed RPC use, so there is one source of
//! truth for OpenAI/custom embeddings) and adapts it to the memory-tree
//! [`Embedder`] trait. Dimensions are pinned to [`EMBEDDING_DIM`] (1024) — the
//! tree's on-disk format is fixed there — and the OpenAI request path now
//! sends the `dimensions` parameter (see `embeddings::openai`) so a reducible
//! model (`text-embedding-3-large`) returns 1024 instead of its native 3072.
//! A returned vector of the wrong size surfaces as the trait's standard
//! "expected N dims" error, which the worker classifies as
//! `embedding_dim_mismatch`.

use anyhow::{Context, Result};
use async_trait::async_trait;

use super::{Embedder, EMBEDDING_DIM};
use crate::openhuman::config::Config;
use crate::openhuman::embeddings::EmbeddingProvider;

/// Adapter from the unified [`EmbeddingProvider`] to the memory-tree
/// [`Embedder`] trait for the OpenAI / custom-OpenAI providers.
pub struct OpenAiCompatEmbedder {
    inner: Box<dyn EmbeddingProvider>,
    /// Short label for logs (e.g. "openai", "custom").
    label: &'static str,
}

impl OpenAiCompatEmbedder {
    /// Try to build the adapter from the user's configured embeddings settings.
    ///
    /// Returns `Ok(None)` when `config.memory.embedding_provider` is **not** an
    /// OpenAI-compatible provider (so the caller's resolution chain continues
    /// to the next branch), and `Ok(Some(_))` when it is. Errors only on an
    /// actual construction failure (which the caller can treat as
    /// fail-fast-worthy).
    ///
    /// Always requests [`EMBEDDING_DIM`] regardless of the user's configured
    /// dimensions — the tree format is fixed at 1024, and the OpenAI path now
    /// honours the `dimensions` param so 3-large complies.
    pub fn try_from_config(config: &Config) -> Result<Option<Self>> {
        let provider = config.memory.embedding_provider.trim();
        let (slug, label): (&str, &'static str) = if provider == "openai" {
            ("openai", "openai")
        } else if provider == "custom" || provider.starts_with("custom:") {
            ("custom", "custom")
        } else {
            // Not an OpenAI-compatible provider — let the caller fall through.
            return Ok(None);
        };

        let model = config.memory.embedding_model.trim();
        let api_key = crate::openhuman::embeddings::resolve_api_key(config, provider);
        let custom_endpoint = provider.strip_prefix("custom:");

        let inner = crate::openhuman::embeddings::create_embedding_provider_with_credentials(
            slug,
            model,
            EMBEDDING_DIM,
            &api_key,
            custom_endpoint,
        )
        .with_context(|| format!("build {label} embedder for memory tree"))?;

        log::debug!(
            "[memory_tree::embed::openai_compat] using {label} provider model={} dims={}",
            model,
            EMBEDDING_DIM
        );
        Ok(Some(Self { inner, label }))
    }
}

#[async_trait]
impl Embedder for OpenAiCompatEmbedder {
    fn name(&self) -> &'static str {
        self.label
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let v = self
            .inner
            .embed_one(text)
            .await
            .with_context(|| format!("{} embeddings failed", self.label))?;
        if v.len() != EMBEDDING_DIM {
            anyhow::bail!(
                "{} embedder returned {} dims, expected {}",
                self.label,
                v.len(),
                EMBEDDING_DIM
            );
        }
        Ok(v)
    }

    /// Collapse N per-text round-trips into a single batched request by
    /// delegating to the inner provider's native batch `embed`. Falls back to
    /// per-text embedding (preserving per-position error attribution) on a
    /// whole-batch failure or a length mismatch.
    async fn embed_batch(&self, texts: &[&str]) -> Vec<Result<Vec<f32>>> {
        super::embed_batch_via_provider(self.inner.as_ref(), self.label, texts).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn cfg_with_provider(p: &str) -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        cfg.config_path = tmp.path().join("config.toml");
        cfg.memory.embedding_provider = p.to_string();
        cfg.memory.embedding_model = "text-embedding-3-large".to_string();
        (tmp, cfg)
    }

    #[test]
    fn none_for_non_openai_providers() {
        // managed / voyage / ollama / none must fall through (Ok(None)).
        for p in ["managed", "cloud", "voyage", "ollama:bge-m3", "none"] {
            let (_tmp, cfg) = cfg_with_provider(p);
            let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
            assert!(got.is_none(), "{p} should fall through, got Some");
        }
    }

    #[test]
    fn some_for_openai() {
        let (_tmp, cfg) = cfg_with_provider("openai");
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("openai should build an adapter");
        assert_eq!(e.name(), "openai");
    }

    #[test]
    fn some_for_custom() {
        let (_tmp, cfg) = cfg_with_provider("custom:https://embed.example/v1");
        let got = OpenAiCompatEmbedder::try_from_config(&cfg).expect("no error");
        let e = got.expect("custom should build an adapter");
        assert_eq!(e.name(), "custom");
    }
}
