//! Snapshot-based change tracking for memory sources.
//!
//! After each sync, this module captures what's in the chunk store for
//! that source, then diffs against previous snapshots to surface
//! additions, removals, and modifications — helping agents understand
//! how their world view has changed over time.
//!
//! Snapshots are built from already-ingested data in `mem_tree_chunks`
//! (not by re-calling source readers), making them free of API calls.
//!
//! Storage is a git repository at `<workspace>/memory_diff/repo` (the diff
//! *ledger*): snapshots are commits, checkpoints are tags, read markers are
//! refs, and diffs are git tree diffs. `mem_tree_chunks` stays authoritative;
//! the ledger is a derived view used purely for change tracking. See
//! [`git_store`] for the mapping.
//!
//! Features:
//! - Per-source snapshots (auto after sync, or manual via RPC)
//! - Diff between any two snapshots
//! - Named checkpoints for cross-source "what changed since X" queries
//! - Agent tool for in-conversation diff queries

pub mod git_store;
pub mod ops;
pub mod rpc;
pub mod schemas;
pub mod tools;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_memory_diff_controller_schemas,
    all_registered_controllers as all_memory_diff_registered_controllers,
};
pub use tools::MemoryDiffTool;
pub use types::{
    ChangeKind, Checkpoint, CrossSourceDiff, DiffResult, DiffSummary, ItemChange, Snapshot,
    SnapshotTrigger,
};
