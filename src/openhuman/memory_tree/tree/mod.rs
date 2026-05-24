//! Generic summary-tree mechanics shared by all three tree flavors:
//! Source (per ingest source), Global (cross-source digest), and Topic
//! (per-entity). Covers storage, buffer management, bucket-seal cascade,
//! time-based flush, and the get-or-create registry primitive.
//!
//! Source-specific policy (the `_source.md` on-disk mirror, the
//! `get_or_create_source_tree` wrapper) lives in the sibling
//! [`crate::openhuman::memory_tree::sources`] module.
//!
//! Global and topic policies (scope constants, hotness gates, curator)
//! live in [`crate::openhuman::memory_tree::global`] and
//! [`crate::openhuman::memory_tree::topic`] respectively; both
//! import generic primitives from this module.
//!
//! Persistence (store + types) has moved to `memory_store::trees`.

pub mod bucket_seal;
pub mod flush;
pub mod registry;

// Re-export persistence from memory_store so callers using tree::store / tree::types still work.
pub use crate::openhuman::memory_store::trees::store;
pub use crate::openhuman::memory_store::trees::types;

pub use crate::openhuman::memory_store::trees::{get_summary_embedding, set_summary_embedding};
pub use crate::openhuman::memory_store::trees::{
    Buffer, SummaryNode, Tree, TreeKind, TreeStatus, INPUT_TOKEN_BUDGET, OUTPUT_TOKEN_BUDGET,
    SUMMARY_FANOUT,
};
pub use bucket_seal::{append_leaf, append_leaf_deferred, LabelStrategy, LeafRef};
pub use registry::{get_or_create_tree, new_summary_id, new_tree_id};
