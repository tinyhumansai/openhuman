//! Memory tree — generic summary-tree engine.
//!
//! This module provides the core tree mechanics: bucket-seal cascades,
//! scoring, embedding, entity extraction, retrieval, and summarisation.
//! It is flavor-agnostic; the specific tree instances (global, topic,
//! source) and their policies live in [`crate::openhuman::memory`].

pub mod graph;
pub mod health;
pub mod ingest;
pub mod io;
pub mod nlp;
pub mod retrieval;
pub mod score;
pub mod summarise;
// `module_inception` is a byproduct of the domain-family reorg: the parent was
// renamed from `memory_tree` to `memory/tree`, which shortened it to match this
// long-standing inner module. Renaming the inner module would be a real rename
// on top of a pure move, so it is allowed here and left as follow-up.
#[allow(clippy::module_inception)]
pub mod tree;
pub mod tree_runtime;

pub use io::{
    TreeLabelStrategy, TreeLeafPayload, TreeReadHit, TreeReadRequest, TreeReadResult,
    TreeWriteOutcome, TreeWriteRequest,
};

// Re-export controller registries.
pub use crate::openhuman::memory::schema::{
    all_controller_schemas as all_memory_tree_controller_schemas,
    all_registered_controllers as all_memory_tree_registered_controllers,
};
pub use crate::openhuman::memory::tree::retrieval::{
    all_retrieval_controller_schemas, all_retrieval_registered_controllers,
};
pub use crate::openhuman::memory::tree::tree_runtime::{
    all_tree_summarizer_controller_schemas, all_tree_summarizer_registered_controllers,
};
