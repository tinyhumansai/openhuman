//! Stable wire types for memory-source diffs.
//!
//! TinyCortex keeps these serde-only values available independently of its
//! git-backed diff engine, so hosts can describe a diff even in a slim build
//! without the `memory-git` feature.

pub use tinycortex::memory::diff::types::{
    ChangeKind, Checkpoint, CrossSourceDiff, DiffResult, DiffSummary, ItemChange, Snapshot,
    SnapshotTrigger,
};
