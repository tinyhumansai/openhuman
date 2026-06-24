//! Memory search tools — all agent-facing retrieval tools consolidated here.
//!
//! New tools are defined here. Existing tools from `memory::query` and
//! `memory_store::tools` are re-exported for a unified import path.

mod chunk_context;
mod hybrid_search;
mod vector_search;

// New tools
pub use chunk_context::MemoryChunkContextTool;
pub use hybrid_search::MemoryHybridSearchTool;
pub use vector_search::MemoryVectorSearchTool;

// Re-export existing tools from memory_store::tools (previously unregistered)
pub use crate::openhuman::memory_store::tools::{
    MemoryStoreKindsTool, MemoryStoreRawChunksTool, MemoryStoreRawSearchTool,
};

// Re-export existing tools from memory::query. The former agentic `walk` /
// `smart_walk` tools are gone — retrieval is now the deterministic
// `fast_retrieve` exposed via the `memory_tree` tool's `walk`/`smart_walk`
// modes (see `memory_tree::retrieval::fast`).
pub use crate::openhuman::memory::query::{
    MemoryTreeDrillDownTool, MemoryTreeFetchLeavesTool, MemoryTreeIngestDocumentTool,
    MemoryTreeQuerySourceTool, MemoryTreeSearchEntitiesTool,
};
