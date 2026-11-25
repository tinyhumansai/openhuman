//! Embedding providers.
//!
//! Converts text into numerical vectors for semantic search (tool discovery,
//! voice, the `embeddings` RPC). Providers:
//!
//! - **Managed** (default): Routes through the managed
//!   backend's OpenAI-compatible embeddings endpoint (Voyage-backed). The recommended path —
//!   works on a fresh install without requiring a local Ollama daemon.
//! - **Voyage**: Direct Voyage AI API with the user's own key.
//! - **OpenAI**: Cloud-based embeddings via the OpenAI API.
//! - **Cohere**: Cohere embed API with the user's own key.
//! - **Ollama**: Local Ollama server. Opt-in for offline-only setups.
//! - **Custom**: Any OpenAI-compatible endpoint.
//! - **Noop**: A fallback provider for keyword-only search.

mod embedding_scope;
mod factory;
mod rpc;
mod schemas;

pub use factory::{
    create_embedding_provider_for_chat, create_embedding_provider_for_inspection,
    create_embedding_provider_with_config, create_embedding_provider_with_credentials,
    default_embedding_provider_for_chat, default_embedding_provider_with_config,
};
pub use rpc::provider_from_config;
pub(crate) use rpc::{probe_dimensions_for_inspection, InspectionEmbeddingError};
/// A shared embedding provider as this module's factories return it, nameable
/// by hosts that hold one without depending on the embeddings crate.
pub type SharedEmbeddingModel = std::sync::Arc<dyn tinyinference_embeddings::EmbeddingModel>;
pub use schemas::{
    all_controller_schemas as all_embeddings_controller_schemas,
    all_registered_controllers as all_embeddings_registered_controllers,
};
#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
