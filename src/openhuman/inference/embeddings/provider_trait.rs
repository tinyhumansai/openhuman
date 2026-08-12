//! Interface for embedding providers that convert text into numerical vectors.
//!
//! [`EmbeddingProvider`] and [`format_embedding_signature`] are **defined in
//! `tinymemory_api::host`** and re-exported here. The extracted memory subsystem
//! takes an `Arc<dyn EmbeddingProvider>` from this host, so the trait has to live
//! somewhere both sides can name — and it has to be *one* trait, not two
//! structurally identical ones, or the trait objects would not be
//! interchangeable.
//!
//! Every existing `inference::embeddings::EmbeddingProvider` path in this crate
//! keeps resolving, and keeps naming the same type.
//!
//! [`TinyAgentsEmbeddingProvider`], the adapter from tinyagents' own
//! embedding-model trait, is re-exported from `tinymemory_core`. It cannot live
//! in the contract crate, which must not depend on tinyagents; and it cannot
//! live here, because the summary tree's embedder factory is core code that
//! builds Ollama models directly and has to wrap them. `tinymemory-core` is the
//! one crate that can name both sides.

pub use tinymemory_api::host::{format_embedding_signature, EmbeddingProvider};
pub use tinymemory_core::embedding_adapter::TinyAgentsEmbeddingProvider;
