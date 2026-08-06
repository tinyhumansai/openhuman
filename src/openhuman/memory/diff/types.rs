//! Domain types for snapshot-based memory-source change tracking — thin host
//! re-export of `tinycortex::memory::diff` types (W7).
//!
//! These are the published RPC/tool wire contract (serde `snake_case` enums +
//! stable field names). The crate port preserves them byte-for-byte, so the
//! host simply re-exports the crate types; the external consumers
//! (`memory_diff::rpc`/`tools`, `subconscious::profiles::memory`, and the RPC
//! controller schemas in `schemas.rs` which reference them by name) keep their
//! `memory_diff::types::*` import paths unchanged.
//!
//! Note: the host types formerly derived `schemars::JsonSchema`, but the RPC
//! surface is described by hand-written [`super::schemas`] (`TypeSchema::Ref`
//! strings), not derived schemas — so the derive was vestigial and its loss is
//! immaterial.

pub use tinycortex::memory::diff::{
    ChangeKind, Checkpoint, CrossSourceDiff, DiffResult, DiffSummary, ItemChange, Snapshot,
    SnapshotTrigger,
};
