//! Factory functions for creating embedding providers.

use std::sync::Arc;

use super::cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
use super::cohere::CohereEmbedding;
use super::provider_trait::EmbeddingProvider;
use super::voyage::VoyageEmbedding;
use super::{NoopEmbedding, OllamaEmbedding, OpenAiEmbedding};

/// Whether to send the OpenAI `dimensions` request-body parameter for this
/// model. Only the `text-embedding-3-*` family honors it (it's how 3-large is
/// pinned to 1024 = `EMBEDDING_DIM`). Sending it to other models or to
/// arbitrary OpenAI-compatible servers (vLLM, text-embeddings-inference,
/// stricter LocalAI builds) makes those servers 400 on an unknown field, so we
/// gate on the model id rather than the provider kind. (Reviewer sanil-23, #3076.)
fn model_supports_dimensions(model: &str) -> bool {
    model.starts_with("text-embedding-3-")
}

/// Creates an embedding provider based on the specified name and configuration.
///
/// Supported provider names:
/// - `"managed"` / `"cloud"` → OpenHuman backend (Voyage-backed) — default
/// - `"voyage"` → direct Voyage AI API (user's own key)
/// - `"openai"` → OpenAI API (user's own key)
/// - `"cohere"` → Cohere API (user's own key)
/// - `"ollama"` → local Ollama server (opt-in for offline-only installs)
/// - `"custom:<url>"` → OpenAI-compatible endpoint
/// - `"none"` → no-op (keyword-only search, no embeddings)
///
/// Returns an error for unrecognised provider names so configuration
/// mistakes surface immediately rather than silently degrading to
/// keyword-only search.
pub fn create_embedding_provider(
    provider: &str,
    model: &str,
    dims: usize,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "cloud" | "managed" => Ok(Box::new(OpenHumanCloudEmbedding::new(
            None, None, true, model, dims,
        ))),
        "voyage" => Ok(Box::new(VoyageEmbedding::new("", model, dims))),
        "ollama" => {
            let base_url = crate::openhuman::inference::local::ollama_base_url();
            Ok(Box::new(OllamaEmbedding::try_new(&base_url, model, dims)?))
        }
        "openai" => Ok(Box::new(
            OpenAiEmbedding::new("https://api.openai.com", "", model, dims)
                .with_send_dimensions(model_supports_dimensions(model))
                .with_required_api_key(true),
        )),
        "cohere" => Ok(Box::new(CohereEmbedding::new("", model, dims))),
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            Ok(Box::new(
                OpenAiEmbedding::new(base_url, "", model, dims)
                    .with_send_dimensions(model_supports_dimensions(model)),
            ))
        }
        "none" => Ok(Box::new(NoopEmbedding)),
        unknown => Err(anyhow::anyhow!(
            "unknown embedding provider: \"{unknown}\". \
             Supported: \"managed\", \"voyage\", \"openai\", \"cohere\", \
             \"ollama\", \"custom:<url>\", \"none\""
        )),
    }
}

/// Creates an embedding provider with explicit API key and endpoint.
///
/// Used by the RPC layer when credentials are loaded from the credential
/// store.
pub fn create_embedding_provider_with_credentials(
    provider: &str,
    model: &str,
    dims: usize,
    api_key: &str,
    custom_endpoint: Option<&str>,
) -> anyhow::Result<Box<dyn EmbeddingProvider>> {
    match provider {
        "cloud" | "managed" => Ok(Box::new(OpenHumanCloudEmbedding::new(
            None, None, true, model, dims,
        ))),
        "voyage" => Ok(Box::new(VoyageEmbedding::new(api_key, model, dims))),
        "ollama" => {
            let base_url = crate::openhuman::inference::local::ollama_base_url();
            Ok(Box::new(OllamaEmbedding::try_new(&base_url, model, dims)?))
        }
        "openai" => Ok(Box::new(
            OpenAiEmbedding::new("https://api.openai.com", api_key, model, dims)
                .with_send_dimensions(model_supports_dimensions(model))
                .with_required_api_key(true),
        )),
        "cohere" => Ok(Box::new(CohereEmbedding::new(api_key, model, dims))),
        "custom" => {
            let url = custom_endpoint.unwrap_or("");
            Ok(Box::new(
                OpenAiEmbedding::new(url, api_key, model, dims)
                    .with_send_dimensions(model_supports_dimensions(model)),
            ))
        }
        name if name.starts_with("custom:") => {
            let url = custom_endpoint.unwrap_or_else(|| name.strip_prefix("custom:").unwrap_or(""));
            Ok(Box::new(
                OpenAiEmbedding::new(url, api_key, model, dims)
                    .with_send_dimensions(model_supports_dimensions(model)),
            ))
        }
        "none" => Ok(Box::new(NoopEmbedding)),
        unknown => Err(anyhow::anyhow!(
            "unknown embedding provider: \"{unknown}\". \
             Supported: \"managed\", \"voyage\", \"openai\", \"cohere\", \
             \"ollama\", \"custom\", \"none\""
        )),
    }
}

/// Returns the default embedding provider — cloud (OpenHuman backend, Voyage).
///
/// The cloud embedder lazily resolves the session JWT and API URL on each
/// call, so this can be constructed before login completes; the first
/// `embed()` will fail with a clear message if the user is unauthenticated.
pub fn default_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(OpenHumanCloudEmbedding::new(
        None,
        None,
        true,
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    ))
}

/// Returns the local Ollama-backed embedding provider. Only used when the
/// caller has explicitly opted into local-only embeddings.
pub fn default_local_embedding_provider() -> Arc<dyn EmbeddingProvider> {
    Arc::new(OllamaEmbedding::default())
}
