//! Memory tree ingestion layer (Phase 1 / issue #707).
//!
//! This is an isolated subdir under `openhuman::memory` implementing the
//! new bucket-seal-ready local memory architecture described in
//! `docs/MEMORY_ARCHITECTURE_LLD.md`. It does **not** share files with the
//! legacy `memory` module; they coexist until the legacy remote-client
//! layer is replaced in a future phase.
//!
//! Phase 1 scope (this module):
//! - source adapters (chat / email / document) → canonical Markdown
//! - chunker with stable deterministic IDs and bounded segments
//! - SQLite persistence with provenance metadata + back-pointer to raw
//! - JSON-RPC controllers under the `memory_tree` namespace
//!
//! Public RPC surface (see `schemas.rs`):
//! - `openhuman.memory_tree_ingest` — unified ingest; caller supplies
//!   `source_kind` (chat|email|document) and a JSON `payload` whose shape
//!   the handler validates based on the kind
//! - `openhuman.memory_tree_list_chunks`
//! - `openhuman.memory_tree_get_chunk`
//!
//! Phases 2-4 (#708 scoring, #709 summary trees, #710 retrieval) build on
//! top of these chunks without modifying the Phase 1 surface.

pub mod global;
pub mod io;
pub mod sources;
pub mod summarise;
pub mod tools;
pub mod topic;
pub mod tree;
pub mod tree_runtime;

pub use io::{
    TreeLabelStrategy, TreeLeafPayload, TreeReadHit, TreeReadRequest, TreeReadResult,
    TreeWriteOutcome, TreeWriteRequest,
};

// Re-export controller registries — moved to memory but keep export names stable.
pub use crate::openhuman::memory::retrieval::{
    all_retrieval_controller_schemas, all_retrieval_registered_controllers,
};
pub use crate::openhuman::memory::schema::{
    all_controller_schemas as all_memory_tree_controller_schemas,
    all_registered_controllers as all_memory_tree_registered_controllers,
};
pub use crate::openhuman::memory_tree::tree_runtime::{
    all_tree_summarizer_controller_schemas, all_tree_summarizer_registered_controllers,
};
