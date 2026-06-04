//! Re-exports the embedding provider trait from the standalone crate.
//!
//! The canonical implementation lives in `openhuman-embeddings`; this module
//! re-exports it so existing in-tree consumers compile unchanged.

pub use openhuman_embeddings::{format_embedding_signature, EmbeddingProvider};
