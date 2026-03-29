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
pub use types::{NamespaceDocumentInput, NamespaceQueryResult};
pub use unified::UnifiedMemory;
