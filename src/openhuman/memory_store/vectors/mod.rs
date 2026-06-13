//! Local vector store (VectorStore) — moved from embeddings::store.
//!
//! Previously at `embeddings::store`. Moved here as part of the
//! memory_store consolidation to co-locate all persistence with memory_store.

pub mod store;

pub use store::{bytes_to_vec, cosine_similarity, vec_to_bytes, SearchResult, VectorStore};
