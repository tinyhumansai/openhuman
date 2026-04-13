//! Embedding providers for the OpenHuman memory system.
//!
//! Converts text into numerical vectors for semantic search. Providers:
//!
//! - **Ollama** (default): Delegates to a local Ollama server — handles model
//!   management, quantization, and GPU acceleration out of the box.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API or compatible endpoints.
//! - **Noop**: A fallback provider for keyword-only search.

pub mod noop;
pub mod ollama;
pub mod openai;
pub mod store;

use std::sync::Arc;

use async_trait::async_trait;

pub use noop::NoopEmbedding;
pub use ollama::{OllamaEmbedding, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL};
pub use openai::OpenAiEmbedding;
pub use store::{bytes_to_vec, cosine_similarity, vec_to_bytes, SearchResult, VectorStore};

/// Interface for embedding providers that convert text into numerical vectors.
#[async_trait]
pub trait EmbeddingProvider: Send + Sync {
    /// Returns the name of the provider (e.g., "ollama", "openai").
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
/// - `"openai"` → OpenAI API
/// - `"custom:<url>"` → OpenAI-compatible endpoint
/// - `"none"` → no-op (keyword-only search, no embeddings)
///
/// Returns an error for unrecognised provider names so configuration
/// mistakes surface immediately rather than silently degrading to
/// keyword-only search.
pub fn create_embedding_provider(
    provider: &str,
    api_key: Option<&str>,
    model: &str,
    dims: usize,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "ollama" => Ok(Box::new(OllamaEmbedding::new("", model, dims))),
        "openai" => {
            let key = api_key.unwrap_or("");
            Ok(Box::new(OpenAiEmbedding::new(
                "https://api.openai.com",
                key,
                model,
                dims,
            )))
        }
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            let key = api_key.unwrap_or("");
            Ok(Box::new(OpenAiEmbedding::new(base_url, key, model, dims)))
        }
        "none" => Ok(Box::new(NoopEmbedding)),
        unknown => Err(anyhow::anyhow!(
            "unknown embedding provider: \"{unknown}\". \
             Supported: \"ollama\", \"openai\", \"custom:<url>\", \"none\""
        )),
    }
}

/// Returns the default local embedding provider (Ollama-backed).
pub fn default_local_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(OllamaEmbedding::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Trait default method ─────────────────────────────────

    #[test]
    fn noop_name_and_dims() {
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

    #[tokio::test]
    async fn noop_embed_one_returns_error() {
        // embed returns empty vec → pop() returns None → error from default impl
        let p = NoopEmbedding;
        let err = p.embed_one("hello").await.unwrap_err();
        assert!(err.to_string().contains("Empty embedding result"));
    }

    #[tokio::test]
    async fn noop_embed_empty_batch() {
        let p = NoopEmbedding;
        let result = p.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    // ── Factory — success ────────────────────────────────────

    #[test]
    fn factory_ollama() {
        let p = create_embedding_provider("ollama", None, DEFAULT_OLLAMA_MODEL, 768).unwrap();
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn factory_openai() {
        let p = create_embedding_provider("openai", Some("key"), "text-embedding-3-small", 1536)
            .unwrap();
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn factory_openai_no_api_key() {
        let p = create_embedding_provider("openai", None, "text-embedding-3-small", 1536).unwrap();
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn factory_custom_url() {
        let p =
            create_embedding_provider("custom:http://localhost:1234", None, "model", 768).unwrap();
        assert_eq!(p.name(), "openai"); // OpenAI-compatible under the hood
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn factory_custom_empty_url() {
        let p = create_embedding_provider("custom:", None, "model", 768).unwrap();
        assert_eq!(p.name(), "openai");
    }

    #[test]
    fn factory_none() {
        let p = create_embedding_provider("none", None, "", 0).unwrap();
        assert_eq!(p.name(), "none");
        assert_eq!(p.dimensions(), 0);
    }

    // ── Factory — errors ─────────────────────────────────────

    #[test]
    fn factory_unknown_provider_errors() {
        let result = create_embedding_provider("cohere", None, "model", 1536);
        let msg = result.err().expect("should be an error").to_string();
        assert!(
            msg.contains("cohere"),
            "should include provider name: {msg}"
        );
        assert!(msg.contains("unknown"), "should say unknown: {msg}");
    }

    #[test]
    fn factory_empty_string_errors() {
        let result = create_embedding_provider("", None, "model", 1536);
        assert!(result
            .err()
            .expect("should error")
            .to_string()
            .contains("unknown"));
    }

    #[test]
    fn factory_fastembed_errors() {
        let result = create_embedding_provider("fastembed", None, "BGESmallENV15", 384);
        assert!(result
            .err()
            .expect("should error")
            .to_string()
            .contains("fastembed"));
    }

    // ── Default provider ─────────────────────────────────────

    #[test]
    fn default_local_provider_uses_ollama() {
        let p = default_local_embedding_provider();
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.dimensions(), DEFAULT_OLLAMA_DIMENSIONS);
    }
}
