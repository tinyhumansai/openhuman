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

// Re-export existing tools from memory::query
pub use crate::openhuman::memory::query::smart_walk::run_smart_walk;
pub use crate::openhuman::memory::query::walk::{
    run_walk, WalkOptions, WalkOutcome, WalkStep, WalkStopReason,
};
pub use crate::openhuman::memory::query::{
    MemoryQueryWalkTool, MemoryTreeDrillDownTool, MemoryTreeFetchLeavesTool,
    MemoryTreeIngestDocumentTool, MemoryTreeQuerySourceTool, MemoryTreeSearchEntitiesTool,
    MemoryTreeWalkTool, SmartMemoryWalkTool, SmartWalkOptions, SmartWalkOutcome, SmartWalkStep,
    SmartWalkStopReason,
};
