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
//! # [`TinyAgentsEmbeddingProvider`] is the host's, not the engine's (#5560)
//!
//! This adapter used to be `pub use tinymemory_core::embedding_adapter::…`, on
//! the reasoning that it could not live in the contract crate (which must not
//! depend on `tinyagents`) and could not live here either, because the memory
//! tree's embedder factory — engine code — also needed to wrap a `tinyagents`
//! model. `tinymemory-core` was the one crate that could name both sides.
//!
//! Only the first half of that was ever a constraint on *this* crate. The host
//! already depends on `tinyagents` directly — every construction site
//! (`factory.rs`, `cloud_adapter.rs`) builds a `tinyagents` `EmbeddingModel` and
//! wraps it here — so the adapter is 40 lines of glue between two dependencies
//! this crate already has, and reaching it through the engine crate was the only
//! thing keeping `tinymemory-core` linked for it.
//!
//! The engine keeps its own copy for its own factory. That is not duplication of
//! *state*: the adapter is a stateless newtype over a boxed trait object, both
//! sides wrap the same `tinyinference::embeddings::EmbeddingModel` (one
//! crate — the root `[patch.crates-io]` points every consumer at
//! `vendor/tinyagents`), and both produce the same
//! `tinymemory_api::host::EmbeddingProvider`. Nothing reads the other's output,
//! which is the same shape the scrubbers, `util::redact` and
//! `memory::obsidian_registry` took when they came home.
//!
//! The signature is load-bearing and is **not** re-derived here:
//! [`format_embedding_signature`] stays the contract's, so a vector written
//! before this move and one written after land in the same embedding space.

use async_trait::async_trait;
use tinyinference::embeddings::EmbeddingModel;

pub use tinymemory_api::host::{format_embedding_signature, EmbeddingProvider};

/// Compatibility adapter from the canonical tinyagents embedding model.
pub struct TinyAgentsEmbeddingProvider {
    model: Box<dyn EmbeddingModel>,
}

impl TinyAgentsEmbeddingProvider {
    pub fn new(model: impl EmbeddingModel + 'static) -> Self {
        Self {
            model: Box::new(model),
        }
    }

    pub fn boxed(model: impl EmbeddingModel + 'static) -> Box<dyn EmbeddingProvider> {
        Box::new(Self::new(model))
    }
}

#[async_trait]
impl EmbeddingProvider for TinyAgentsEmbeddingProvider {
    fn name(&self) -> &str {
        self.model.name()
    }

    fn model_id(&self) -> &str {
        self.model.model_id()
    }

    fn dimensions(&self) -> usize {
        self.model.dimensions()
    }

    fn signature(&self) -> String {
        self.model.signature()
    }

    /// The one shape difference between the two traits: the contract takes
    /// `&[&str]` and `EmbeddingModel` takes `&[String]`, so the batch is owned
    /// here. Kept verbatim from the engine-side adapter this replaces — a
    /// "cheaper" variant that embedded one text at a time would turn one
    /// provider request into N.
    async fn embed(&self, texts: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        let owned = texts
            .iter()
            .map(|text| (*text).to_owned())
            .collect::<Vec<_>>();
        self.model
            .embed(&owned)
            .await
            .map_err(|error| anyhow::anyhow!(error))
    }
}
