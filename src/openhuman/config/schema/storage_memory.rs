//! Storage provider and memory configuration.
//!
//! **The definitions moved to [`tinymemory_api::host`].** The memory subsystem
//! itself was extracted into `tinymemory-core`, which reads these fields
//! directly; leaving them here would have meant a trait accessor per field.
//!
//! Their serde representation is persisted in users' `config.toml`, so field
//! names, defaults and `#[serde(...)]` attributes are a compatibility surface —
//! change them there, deliberately, not incidentally.
//!
//! This module re-exports them so every existing `config::schema::…` path in
//! this crate keeps resolving.

pub use tinymemory_api::host::storage_memory::{
    LlmBackend, MemoryConfig, MemoryTreeConfig, StorageConfig, StorageProviderConfig,
    StorageProviderSection, DEFAULT_CLOUD_LLM_MODEL,
};
