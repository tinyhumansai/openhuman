//! Host layer over what used to be `tinymemory_core::tree::tree`.
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//!
//! # The `pub use tinymemory_core::tree::tree::*` shim is gone (#5560)
//!
//! This file carried a glob re-export so every historical
//! `memory::tree::tree::…` path kept resolving — the engine-owned persistence
//! types (`Tree`, `SummaryNode`, `Buffer`, `TreeKind`, the seal/flush/registry
//! mechanics). Its production surface was re-measured at empty: the doc claim
//! that `read_rpc::admin`'s `flush_source_tree` needed `TreeFactory` and
//! `force_flush_tree` had gone stale — that handler asks the bound driver's
//! `MemoryTree::flush_source_tree` now, and the `TreeFactory` mentions left in
//! `admin.rs` are prose about the old shape, not code. What the glob still
//! served was tests (`integrations::composio::ops_tests`, the recap's
//! `#[cfg(test)]` arm, `tests/raw_coverage/`) reaching `bucket_seal`,
//! `store`, `registry`, `flush` and `TreeKind`/`TreeStatus`.
//!
//! A production `pub use` that exists only to serve a test is exactly what
//! keeps a crate in `[dependencies]`, so those tests name
//! `tinymemory_core::tree::tree::…` directly now — served by the
//! `[dev-dependencies]` entry, the way the earlier-repointed `tests/` targets
//! already were — and the glob is deleted. `MemoryTree` on the contract stays
//! namespace-addressed (append / query_source / drill_down / seal / cascade /
//! summary_forest / recent_leaves) with no door onto a tree *object*, which is
//! why the tests still want the engine crate rather than the driver.
//!
//! Worth keeping from the old comment, because two same-named types sit one
//! module apart: the engine's `tree::tree::TreeStatus` (`store::trees`) is a
//! **different type** from the contract's `TreeStatus`, which the sibling
//! `tree_runtime` module exports. Check which one a call site means before
//! moving it.

pub mod canonicalize_types;
pub mod rpc;
