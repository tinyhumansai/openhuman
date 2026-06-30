//! Retrieval tools for the hierarchical memory tree (#710).
//!
//! Exposes the **source** trees as LLM-callable primitives. Each tool is
//! deterministic and scope-specific; orchestration (which tool to call, how
//! to combine results) is left to the calling LLM — there is no classifier,
//! gate, or composer here. The global (time-axis) and topic (subject-axis)
//! trees were removed: source trees hold all the content, and walking the
//! source hierarchy plus the entity index reconstructs both projections.
//!
//! Public JSON-RPC surface (see `schemas.rs`):
//! - `openhuman.memory_tree_query_source`   — per-source summary retrieval
//! - `openhuman.memory_tree_search_entities` — fuzzy canonical-id lookup
//! - `openhuman.memory_tree_drill_down`     — walk summary children
//! - `openhuman.memory_tree_fetch_leaves`   — batch chunk hydration
//! - `openhuman.memory_tree_cover_window`   — minimum-node cover of a window
//!
//! All tools share the [`types::RetrievalHit`] / [`types::QueryResponse`]
//! shape so the LLM sees a uniform schema regardless of which tool ran.

pub mod cover;
pub mod drill_down;
pub mod fast;
pub mod fetch;
pub mod rpc;
pub mod schemas;
pub mod search;
pub mod source;
pub mod types;

#[cfg(test)]
mod benchmarks;
#[cfg(test)]
mod integration_tests;

pub use cover::cover_window;
pub use drill_down::drill_down;
pub use fast::{fast_retrieve, FastRetrieveOptions};
pub use fetch::fetch_leaves;
pub use schemas::{
    all_controller_schemas as all_retrieval_controller_schemas,
    all_registered_controllers as all_retrieval_registered_controllers,
};
pub use search::search_entities;
pub use source::query_source;
pub use types::{EntityMatch, NodeKind, QueryResponse, RetrievalHit};
