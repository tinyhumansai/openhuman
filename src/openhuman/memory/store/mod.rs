//! Namespace memory store (SQLite + markdown + vector chunks).

pub mod types;
mod unified;

mod client;
mod factories;
mod memory_trait;

pub use client::{MemoryClient, MemoryClientRef, MemoryState};
pub use factories::{
    create_memory, create_memory_for_migration, create_memory_with_storage,
    create_memory_with_storage_and_routes, effective_memory_backend_name,
};
pub use types::{
    GraphRelationRecord, MemoryItemKind, MemoryKvRecord, NamespaceDocumentInput,
    NamespaceMemoryHit, NamespaceQueryResult, NamespaceRetrievalContext, RetrievalScoreBreakdown,
    StoredMemoryDocument,
};
pub use unified::events;
pub use unified::fts5;
pub use unified::profile;
pub use unified::segments;
pub use unified::UnifiedMemory;
