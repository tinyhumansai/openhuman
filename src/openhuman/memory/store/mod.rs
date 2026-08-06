//! # Memory Store
//!
//! This module provides the core storage abstractions and implementations for
//! the OpenHuman memory system. It manages namespaces, documents, text chunks,
//! vector embeddings, and graph relations.
//!
//! The memory system is designed to be pluggable, with the primary implementation
//! being `UnifiedMemory`, which uses SQLite for structured data and Full-Text
//! Search (FTS5), along with vector storage for semantic retrieval.
//!
//! ## Submodules
//!
//! - `types`: Common data structures and types used across the memory store.
//! - `namespace_store`: Host-retained SQLite namespace documents, graph,
//!   episodic/event/segment/profile tables, and their query policy.
//! - `client`: High-level client interface for interacting with the memory system.
//! - `factories`: Factory functions for creating and initializing memory instances.
//! - `memory_trait`: Defines the `Memory` trait that all implementations must satisfy.

pub mod chunks;
pub mod content;
pub mod entities;
pub mod kinds;
pub mod kv;
pub mod namespace_store;
pub mod retrieval;
pub mod safety;
pub mod tools;
pub mod traits;
pub mod trees;
pub mod types;
pub mod vectors;

mod client;
pub mod factories;
mod memory_trait;

pub use kinds::MemoryKind;
pub use traits::{ObsidianFile, ObsidianRepresentable, VectorEmbeddable};

pub use client::{MemoryClient, MemoryClientRef, MemoryState};
pub use factories::{
    active_embedding_signature, create_memory, create_memory_for_migration,
    create_memory_with_local_ai, effective_embedding_settings, effective_memory_backend_name,
};
pub use namespace_store::events;
pub use namespace_store::fts5;
pub use namespace_store::profile;
pub use namespace_store::segments;
pub use namespace_store::UnifiedMemory;
pub use types::{
    GraphRelationRecord, MemoryItemKind, MemoryKvRecord, NamespaceDocumentInput,
    NamespaceMemoryHit, NamespaceQueryResult, NamespaceRetrievalContext, RetrievalScoreBreakdown,
    StoredMemoryDocument,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_reexports_expected_memory_kind_catalog() {
        assert!(MemoryKind::ALL.contains(&MemoryKind::Chunk));
        assert!(MemoryKind::ALL.contains(&MemoryKind::Tree));
        assert!(MemoryKind::ALL.contains(&MemoryKind::Contact));
    }
}
