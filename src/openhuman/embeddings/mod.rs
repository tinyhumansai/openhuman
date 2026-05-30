//! Embedding providers for the OpenHuman memory system.
//!
//! Converts text into numerical vectors for semantic search. Providers:
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

pub mod catalog;
pub mod cloud;
pub mod cohere;
mod factory;
pub mod noop;
pub mod ollama;
pub mod openai;
mod provider_trait;
pub mod rate_limit;
pub mod retry_after;
mod rpc;
mod schemas;
pub mod voyage;

// VectorStore has moved to memory_store::vectors; re-exported for callers.
pub use crate::openhuman::memory_store::vectors::store;

pub use crate::openhuman::memory_store::vectors::{
    bytes_to_vec, cosine_similarity, vec_to_bytes, SearchResult, VectorStore,
};
pub use cloud::{
    OpenHumanCloudEmbedding, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL,
};
pub use factory::{
    create_embedding_provider, default_embedding_provider, default_local_embedding_provider,
};
pub use noop::NoopEmbedding;
pub use ollama::{OllamaEmbedding, DEFAULT_OLLAMA_DIMENSIONS, DEFAULT_OLLAMA_MODEL};
pub use openai::OpenAiEmbedding;
pub use provider_trait::{format_embedding_signature, EmbeddingProvider};
pub use rpc::provider_from_config;
pub use schemas::{
    all_controller_schemas as all_embeddings_controller_schemas,
    all_registered_controllers as all_embeddings_registered_controllers,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
