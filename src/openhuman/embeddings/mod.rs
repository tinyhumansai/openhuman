//! Embedding providers for the OpenHuman memory system.
//!
//! This module provides a unified interface for converting text into vector
//! embeddings. Providers (in priority order):
//!
//! - **Ollama** (default): Delegates to a local Ollama server — handles model
//!   management, quantization, and GPU acceleration out of the box.
//! - **Candle**: Pure-Rust in-process inference via HuggingFace's candle
//!   framework. No external process required, but heavier on CPU.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API or compatible endpoints.
//! - **Noop**: A fallback provider for keyword-only search.

pub mod candle_embed;
pub mod noop;
pub mod ollama;
pub mod openai;

use std::sync::Arc;

use async_trait::async_trait;

pub use candle_embed::CandleEmbedding;
pub use noop::NoopEmbedding;
pub use ollama::{OllamaEmbedding, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL};
pub use openai::OpenAiEmbedding;

// Legacy constant aliases so existing config references keep compiling.
pub const DEFAULT_FASTEMBED_MODEL: &str = DEFAULT_OLLAMA_MODEL;
pub const DEFAULT_FASTEMBED_DIMENSIONS: usize = DEFAULT_OLLAMA_DIMENSIONS;

/// Interface for embedding providers that convert text into numerical vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns the name of the provider (e.g., "ollama", "candle", "openai").
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
/// Supported provider names:
/// - `"ollama"` → local Ollama server (default, preferred)
/// - `"fastembed"` or `"candle"` → in-process candle inference
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
        "ollama" => Box::new(OllamaEmbedding::new("", model, dims)),
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

/// Returns the default local embedding provider (Ollama-backed).
pub fn default_local_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(OllamaEmbedding::default())
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
    fn factory_ollama() {
        let p = create_embedding_provider("ollama", None, DEFAULT_OLLAMA_MODEL, 768);
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn factory_candle() {
        let p = create_embedding_provider(
            "candle",
            None,
            candle_embed::DEFAULT_MODEL_NAME,
            384,
        );
        assert_eq!(p.name(), "candle");
        assert_eq!(p.dimensions(), 384);
    }

    #[test]
    fn factory_fastembed_compat() {
        // Legacy "fastembed" config value maps to candle provider.
        let p = create_embedding_provider("fastembed", None, "BGESmallENV15", 384);
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
    fn default_local_provider_uses_ollama() {
        let p = default_local_embedding_provider();
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.dimensions(), DEFAULT_OLLAMA_DIMENSIONS);
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
