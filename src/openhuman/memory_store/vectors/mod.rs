//! TinyCortex-owned local vector storage.

pub use tinycortex::memory::store::vectors::{
    bytes_to_vec, cosine_similarity, format_embedding_signature, vec_to_bytes, EmbeddingBackend,
    InertEmbedding, SearchResult, VectorStore,
};
