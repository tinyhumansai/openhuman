//! Phase 4 — retrieval tools for the hierarchical memory tree (#710).
//!
//! Exposes the source / global / topic trees produced by Phase 3 as six
//! LLM-callable primitives. Each tool is deterministic and scope-specific;
//! orchestration (which tool to call, how to combine results) is left to
//! the calling LLM — there is no classifier, gate, or composer in this
//! phase.
//!
//! Public JSON-RPC surface (see `schemas.rs`):
//! - `openhuman.memory_tree_query_source`   — per-source summary retrieval
//! - `openhuman.memory_tree_query_global`   — cross-source digest for a window
//! - `openhuman.memory_tree_query_topic`    — entity-scoped retrieval
//! - `openhuman.memory_tree_search_entities` — fuzzy canonical-id lookup
//! - `openhuman.memory_tree_drill_down`     — walk summary children
//! - `openhuman.memory_tree_fetch_leaves`   — batch chunk hydration
//!
//! All tools share the [`types::RetrievalHit`] / [`types::QueryResponse`]
//! shape so the LLM sees a uniform schema regardless of which tool ran.

pub mod drill_down;
pub mod fetch;
pub mod global;
pub mod rpc;
pub mod schemas;
pub mod search;
pub mod source;
pub mod topic;
pub mod types;

#[cfg(test)]
mod benchmarks;
#[cfg(test)]
mod integration_test;

pub use drill_down::drill_down;
pub use fetch::fetch_leaves;
pub use global::query_global;
pub use schemas::{
    all_controller_schemas as all_retrieval_controller_schemas,
    all_registered_controllers as all_retrieval_registered_controllers,
};
pub use search::search_entities;
pub use source::query_source;
pub use topic::query_topic;
pub use types::{EntityMatch, NodeKind, QueryResponse, RetrievalHit};
