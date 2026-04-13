//! Embedding providers for the OpenHuman memory system.
//!
//! This module provides a unified interface for converting text into vector
//! embeddings. It supports multiple providers:
//! - **Candle**: Pure-Rust local embeddings using HuggingFace's candle framework.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API or compatible endpoints.
//! - **Noop**: A fallback provider for keyword-only search.

pub mod candle_embed;
pub mod noop;
pub mod openai;

use std::sync::Arc;

use async_trait::async_trait;

pub use candle_embed::{CandleEmbedding, DEFAULT_DIMENSIONS, DEFAULT_HF_REPO, DEFAULT_MODEL_NAME};
pub use noop::NoopEmbedding;
pub use openai::OpenAiEmbedding;

// Re-export legacy constant names so existing config references keep working.
pub const DEFAULT_FASTEMBED_MODEL: &str = DEFAULT_MODEL_NAME;
pub const DEFAULT_FASTEMBED_DIMENSIONS: usize = DEFAULT_DIMENSIONS;

/// Interface for embedding providers that convert text into numerical vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns the name of the provider (e.g., "candle", "openai").
    fn name(&self) -> &str;

    /// Returns the number of dimensions in the generated embeddings.
    fn dimensions(&self) -> usize;

    /// Generates embeddings for a batch of strings.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>>;

    /// Generates an embedding for a single string.
    async fn embed_one(&self, text: &str) -> anyhow::Result<Vec<f32>> {
        let mut results = self.embed(&[text]).await?;
        results
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Empty embedding result"))
    }
}

// ── Factory ──────────────────────────────────────────────────

/// Creates an embedding provider based on the specified name and configuration.
///
/// Supports:
/// - `"fastembed"` or `"candle"` → local candle-based embeddings
/// - `"openai"` → OpenAI API
/// - `"custom:<url>"` → OpenAI-compatible endpoint
/// - anything else → no-op (keyword-only search)
pub fn create_embedding_provider(
    provider: &str,
    api_key: Option<&str>,
    model: &str,
    dims: usize,
) -> Box<dyn EmbeddingProvider> {
    match provider {
        // Accept both old name and new name for backwards compatibility.
        "fastembed" | "candle" => Box::new(CandleEmbedding::new(model, dims)),
        "openai" => {
            let key = api_key.unwrap_or("");
            Box::new(OpenAiEmbedding::new(
                "https://api.openai.com",
                key,
                model,
                dims,
            ))
        }
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            let key = api_key.unwrap_or("");
            Box::new(OpenAiEmbedding::new(base_url, key, model, dims))
        }
        _ => Box::new(NoopEmbedding),
    }
}

/// Returns the default local embedding provider (candle-based).
pub fn default_local_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(CandleEmbedding::new(DEFAULT_MODEL_NAME, DEFAULT_DIMENSIONS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noop_name() {
        let p = NoopEmbedding;
        assert_eq!(p.name(), "none");
        assert_eq!(p.dimensions(), 0);
    }

    #[tokio::test]
    async fn noop_embed_returns_empty() {
        let p = NoopEmbedding;
        let result = p.embed(&["hello"]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn factory_none() {
        let p = create_embedding_provider("none", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn factory_openai() {
        let p = create_embedding_provider("openai", Some("key"), "text-embedding-3-small", 1536);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn factory_candle() {
        let p = create_embedding_provider("candle", None, DEFAULT_MODEL_NAME, 384);
        assert_eq!(p.name(), "candle");
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn factory_fastembed_compat() {
        // "fastembed" config value still works — maps to candle provider.
        let p = create_embedding_provider("fastembed", None, DEFAULT_MODEL_NAME, 384);
        assert_eq!(p.name(), "candle");
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn factory_custom_url() {
        let p = create_embedding_provider("custom:http://localhost:1234", None, "model", 768);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 768);
    }

    #[tokio::test]
    async fn noop_embed_one_returns_error() {
        let p = NoopEmbedding;
        let result = p.embed_one("hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn noop_embed_empty_batch() {
        let p = NoopEmbedding;
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn factory_empty_string_returns_noop() {
        let p = create_embedding_provider("", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn factory_unknown_provider_returns_noop() {
        let p = create_embedding_provider("cohere", None, "model", 1536);
        assert_eq!(p.name(), "none");
    }

    #[test]
    fn default_local_provider_uses_candle() {
        let p = default_local_embedding_provider();
        assert_eq!(p.name(), "candle");
        assert_eq!(p.dimensions(), DEFAULT_DIMENSIONS);
    }

    #[test]
    fn factory_custom_empty_url() {
        let p = create_embedding_provider("custom:", None, "model", 768);
        assert_eq!(p.name(), "openai");
    }

    #[test]
    fn factory_openai_no_api_key() {
        let p = create_embedding_provider("openai", None, "text-embedding-3-small", 1536);
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }
}
