//! Embedding providers for semantic search.
//!
//! This crate provides the [`EmbeddingProvider`] trait and multiple
//! implementations (OpenAI-compatible, Ollama, Cohere, Voyage, Noop) that
//! convert text into numerical vectors for similarity search.
//!
//! # Usage
//!
//! ```rust,no_run
//! use openhuman_embeddings::{EmbeddingProvider, OpenAiEmbedding};
//!
//! # async fn example() -> anyhow::Result<()> {
//! let provider = OpenAiEmbedding::new("https://api.openai.com", "sk-...", "text-embedding-3-small", 1536);
//! let vectors = provider.embed(&["hello world"]).await?;
//! # Ok(())
//! # }
//! ```

pub mod catalog;
pub mod cohere;
pub mod noop;
pub mod ollama;
pub mod openai;
pub mod rate_limit;
pub mod retry_after;
pub mod voyage;

mod provider_trait;

pub use provider_trait::{format_embedding_signature, EmbeddingProvider};

pub use catalog::{
    all_providers, default_model_for, find_model, find_provider, EmbeddingModelPreset,
    EmbeddingProviderEntry, PROVIDER_COHERE, PROVIDER_CUSTOM, PROVIDER_MANAGED, PROVIDER_NONE,
    PROVIDER_OLLAMA, PROVIDER_OPENAI, PROVIDER_VOYAGE,
};
pub use cohere::CohereEmbedding;
pub use noop::NoopEmbedding;
pub use ollama::{OllamaEmbedding, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL};
pub use openai::OpenAiEmbedding;
pub use voyage::VoyageEmbedding;

/// Creates an embedding provider based on the specified name and configuration.
///
/// # Arguments
/// - `provider` — Provider slug (e.g. `"openai"`, `"ollama"`, `"cohere"`, `"voyage"`, `"custom:<url>"`, `"none"`)
/// - `model` — Model ID
/// - `dims` — Expected embedding dimensions
/// - `api_key` — API key (empty string if not needed)
/// - `ollama_base_url` — Base URL for Ollama (only used when `provider == "ollama"`)
/// - `http_client` — Optional pre-configured `reqwest::Client`
///
/// Note: The `"managed"` / `"cloud"` provider is NOT handled here — it requires
/// host-app credentials. The host app should handle that case before delegating
/// to this function.
pub fn create_provider(
    provider: &str,
    model: &str,
    dims: usize,
    api_key: &str,
    ollama_base_url: &str,
    http_client: Option<reqwest::Client>,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    let client = http_client.unwrap_or_else(reqwest::Client::new);
    match provider {
        "ollama" => Ok(Box::new(
            OllamaEmbedding::try_new(ollama_base_url, model, dims)?.with_client(client),
        )),
        "openai" => Ok(Box::new(
            OpenAiEmbedding::new("https://api.openai.com", api_key, model, dims)
                .with_send_dimensions(model.starts_with("text-embedding-3-"))
                .with_client(client),
        )),
        "voyage" => Ok(Box::new(
            VoyageEmbedding::new(api_key, model, dims).with_client(client),
        )),
        "cohere" => Ok(Box::new(
            CohereEmbedding::new(api_key, model, dims).with_client(client),
        )),
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            Ok(Box::new(
                OpenAiEmbedding::new(base_url, api_key, model, dims)
                    .with_send_dimensions(model.starts_with("text-embedding-3-"))
                    .with_client(client),
            ))
        }
        "custom" => {
            // When "custom" without URL suffix, use ollama_base_url as custom endpoint
            Ok(Box::new(
                OpenAiEmbedding::new(ollama_base_url, api_key, model, dims)
                    .with_send_dimensions(model.starts_with("text-embedding-3-"))
                    .with_client(client),
            ))
        }
        "none" => Ok(Box::new(NoopEmbedding)),
        unknown => Err(anyhow::anyhow!(
            "unknown embedding provider: \"{unknown}\". \
             Supported: \"openai\", \"voyage\", \"cohere\", \"ollama\", \"custom:<url>\", \"none\""
        )),
    }
}
