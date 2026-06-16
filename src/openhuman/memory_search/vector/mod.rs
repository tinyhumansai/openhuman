//! Vector store, cosine similarity, and diversity algorithms.

pub mod mmr;
pub mod store;

pub use store::{bytes_to_vec, cosine_similarity, vec_to_bytes, SearchResult, VectorStore};
