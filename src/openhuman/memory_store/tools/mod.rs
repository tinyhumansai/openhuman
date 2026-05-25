//! Raw search/retrieve tools surfaced to the agent harness.
//!
//! These tools expose the storage layer directly — no policy, no scoring
//! beyond what the underlying backend already applies. They exist so an agent
//! can drop one layer below the curated `memory_tree_*` tools when it needs
//! to inspect or operate on raw memory_store rows.
//!
//! Three tools, one per major access pattern:
//! - [`MemoryStoreRawSearchTool`]  — hybrid (vector+keyword) namespace query.
//! - [`MemoryStoreRawChunksTool`]  — structured chunk filter by source/owner/
//!   time/tags.
//! - [`MemoryStoreKindsTool`]      — introspection: enumerate every
//!   [`MemoryKind`] the store supports.
//!
//! All three are async, return JSON, and follow the project Tool trait.

mod kinds;
mod raw_chunks;
mod raw_search;

pub use kinds::MemoryStoreKindsTool;
pub use raw_chunks::MemoryStoreRawChunksTool;
pub use raw_search::MemoryStoreRawSearchTool;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::Tool;

    #[test]
    fn exports_memory_store_tools_with_stable_names() {
        assert_eq!(MemoryStoreKindsTool.name(), "memory_store_kinds");
        assert_eq!(MemoryStoreRawChunksTool.name(), "memory_store_raw_chunks");
        assert_eq!(MemoryStoreRawSearchTool.name(), "memory_store_raw_search");
    }
}
