//! Embedding providers for the OpenHuman memory system.
//!
//! Converts text into numerical vectors for semantic search. Providers:
//!
//! - **Ollama** (default): Delegates to a local Ollama server — handles model
//!   management, quantization, and GPU acceleration out of the box.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API or compatible endpoints.
//! - **Noop**: A fallback provider for keyword-only search.

mod factory;
pub mod noop;
pub mod ollama;
pub mod openai;
mod provider_trait;
pub mod store;

pub use factory::{create_embedding_provider, default_local_embedding_provider};
pub use noop::NoopEmbedding;
pub use ollama::{OllamaEmbedding, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL};
pub use openai::OpenAiEmbedding;
pub use provider_trait::EmbeddingProvider;
pub use store::{bytes_to_vec, cosine_similarity, vec_to_bytes, SearchResult, VectorStore};

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
        let p = create_embedding_provider("ollama", DEFAULT_OLLAMA_MODEL, 768).unwrap();
        assert_eq!(p.name(), "ollama");
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn factory_openai() {
        let p = create_embedding_provider("openai", "text-embedding-3-small", 1536).unwrap();
        assert_eq!(p.name(), "openai");
        assert_eq!(p.dimensions(), 1536);
    }

    #[test]
    fn factory_custom_url() {
        let p = create_embedding_provider("custom:http://localhost:1234", "model", 768).unwrap();
        assert_eq!(p.name(), "openai"); // OpenAI-compatible under the hood
        assert_eq!(p.dimensions(), 768);
    }

    #[test]
    fn factory_custom_empty_url() {
        let p = create_embedding_provider("custom:", "model", 768).unwrap();
        assert_eq!(p.name(), "openai");
    }

    #[test]
    fn factory_none() {
        let p = create_embedding_provider("none", "", 0).unwrap();
        assert_eq!(p.name(), "none");
        assert_eq!(p.dimensions(), 0);
    }

    // ── Factory — errors ─────────────────────────────────────

    #[test]
    fn factory_unknown_provider_errors() {
        let result = create_embedding_provider("cohere", "model", 1536);
        let msg = result.err().expect("should be an error").to_string();
        assert!(
            msg.contains("cohere"),
            "should include provider name: {msg}"
        );
        assert!(msg.contains("unknown"), "should say unknown: {msg}");
    }

    #[test]
    fn factory_empty_string_errors() {
        let result = create_embedding_provider("", "model", 1536);
        assert!(result
            .err()
            .expect("should error")
            .to_string()
            .contains("unknown"));
    }

    #[test]
    fn factory_fastembed_errors() {
        let result = create_embedding_provider("fastembed", "BGESmallENV15", 384);
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
