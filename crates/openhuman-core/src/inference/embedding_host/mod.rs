//! Embedding providers.
//!
//! Converts text into numerical vectors for semantic search (tool discovery,
//! voice, the `embeddings` RPC). Providers:
//!
//! - **Managed** (default): Routes through the OpenHuman backend's
//!   `POST /openai/v1/embeddings` (Voyage-backed). The recommended path —
//!   works on a fresh install without requiring a local Ollama daemon.
//! - **Voyage**: Direct Voyage AI API with the user's own key.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API.
//! - **Cohere**: Cohere embed API with the user's own key.
//! - **Ollama**: Local Ollama server. Opt-in for offline-only setups.
//! - **Custom**: Any OpenAI-compatible endpoint.
//! - **Noop**: A fallback provider for keyword-only search.

#[path = "cloud_adapter.rs"]
pub mod cloud;
mod embedding_trait;
mod factory;
mod provider_trait;
mod rpc;
mod schemas;

pub use cloud::{
    OpenHumanCloudEmbeddingModel, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
pub use factory::{
    create_embedding_provider_with_config, create_embedding_provider_with_credentials,
    default_embedding_provider_with_config,
};
pub use provider_trait::{
    format_embedding_signature, EmbeddingProvider, TinyInferenceEmbeddingProvider,
};
pub use rpc::provider_from_config;
pub use schemas::{
    all_controller_schemas as all_embeddings_controller_schemas,
    all_registered_controllers as all_embeddings_registered_controllers,
};
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
