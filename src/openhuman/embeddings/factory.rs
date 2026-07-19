//! Factory functions for creating embedding providers.

use std::sync::Arc;

use super::cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
use super::provider_trait::{EmbeddingProvider, TinyAgentsEmbeddingProvider};
use tinyagents::harness::embeddings::{
    CohereEmbeddingModel, NoopEmbeddingModel, OllamaEmbeddingModel, OpenAiEmbeddingModel,
    VoyageEmbeddingModel,
};

fn openai_model(
    base_url: &str,
    api_key: &str,
    model: &str,
    dims: usize,
    required_key: bool,
) -> OpenAiEmbeddingModel {
    OpenAiEmbeddingModel::new(api_key)
        .with_base_url(base_url)
        .with_model(model)
        .with_dimensions(dims)
        .with_send_dimensions(model_supports_dimensions(model))
        .with_required_api_key(required_key)
}

/// Whether to send the OpenAI `dimensions` request-body parameter for this
/// model. Only the `text-embedding-3-*` family honors it (it's how 3-large is
/// pinned to 1024 = `EMBEDDING_DIM`). Sending it to other models or to
/// arbitrary OpenAI-compatible servers (vLLM, text-embeddings-inference,
/// stricter LocalAI builds) makes those servers 400 on an unknown field, so we
/// gate on the model id rather than the provider kind. (Reviewer sanil-23, #3076.)
pub(crate) fn model_supports_dimensions(model: &str) -> bool {
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
        "voyage" => Ok(TinyAgentsEmbeddingProvider::boxed(
            VoyageEmbeddingModel::with_options(
                "",
                model,
                dims,
                tinyagents::harness::embeddings::VOYAGE_API_BASE,
            ),
        )),
        "ollama" => {
            let base_url = crate::openhuman::inference::local::ollama_base_url();
            Ok(TinyAgentsEmbeddingProvider::boxed(
                OllamaEmbeddingModel::try_new(&base_url, model, dims)?,
            ))
        }
        "openai" => Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
            "https://api.openai.com",
            "",
            model,
            dims,
            true,
        ))),
        "cohere" => Ok(TinyAgentsEmbeddingProvider::boxed(
            CohereEmbeddingModel::new("")
                .with_model(model)
                .with_dimensions(dims),
        )),
        name if name.starts_with("custom:") => {
            let base_url = name.strip_prefix("custom:").unwrap_or("");
            Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
                base_url, "", model, dims, false,
            )))
        }
        "none" => Ok(TinyAgentsEmbeddingProvider::boxed(NoopEmbeddingModel)),
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
        "voyage" => Ok(TinyAgentsEmbeddingProvider::boxed(
            VoyageEmbeddingModel::with_options(
                api_key,
                model,
                dims,
                tinyagents::harness::embeddings::VOYAGE_API_BASE,
            ),
        )),
        "ollama" => {
            let base_url = crate::openhuman::inference::local::ollama_base_url();
            Ok(TinyAgentsEmbeddingProvider::boxed(
                OllamaEmbeddingModel::try_new(&base_url, model, dims)?,
            ))
        }
        "openai" => Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
            "https://api.openai.com",
            api_key,
            model,
            dims,
            true,
        ))),
        "cohere" => Ok(TinyAgentsEmbeddingProvider::boxed(
            CohereEmbeddingModel::new(api_key)
                .with_model(model)
                .with_dimensions(dims),
        )),
        "custom" => {
            let url = custom_endpoint.unwrap_or("");
            Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
                url, api_key, model, dims, false,
            )))
        }
        name if name.starts_with("custom:") => {
            let url = custom_endpoint.unwrap_or_else(|| name.strip_prefix("custom:").unwrap_or(""));
            Ok(TinyAgentsEmbeddingProvider::boxed(openai_model(
                url, api_key, model, dims, false,
            )))
        }
        "none" => Ok(TinyAgentsEmbeddingProvider::boxed(NoopEmbeddingModel)),
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
    Arc::new(TinyAgentsEmbeddingProvider::new(
        OllamaEmbeddingModel::default(),
    ))
}
