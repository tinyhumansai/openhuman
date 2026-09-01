//! Host layer over [`tinymemory_core::tree::retrieval`].
//!
//! The domain itself lives in the extracted crate; what stays here is its
//! JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//!
//! # The engine glob is gone — it had no production caller left (#5560)
//!
//! This module used to open with `pub use tinymemory_core::tree::retrieval::*`,
//! carried by a comment listing what kept it alive: `fast_retrieve` +
//! `FastRetrieveOptions` from `memory::agent::ops` and
//! `memory::schema::handlers`, `types::{NodeKind, QueryResponse,
//! RetrievalHit}` from `read_rpc::chunks` and the sub-agent runner, and
//! `source::{SourceQuery, query_source_scoped}`.
//!
//! **That list had gone stale, every entry of it.** Each of those call sites
//! now takes the contract instead —
//! `crate::openhuman::memory::api::provider::retrieval::{FastRetrieveQuery,
//! RetrievalResponse, …}` — and `rpc.rs` and `schemas.rs` in this directory
//! name nothing from the engine either. Deleting the line and compiling the
//! lib produced **zero** errors, which is the check to repeat before trusting
//! any sentence in this file: grep finds the path
//! `memory::tree::retrieval::…`, but almost every hit is `retrieval::rpc`,
//! the `pub mod` below, and a glob is invisible to grep anyway.
//!
//! The surviving callers were all tests, and they name
//! `tinymemory_core::tree::retrieval::…` directly now: the engine crate is a
//! dev-dependency, and a dev-dependency does not keep a crate in the shipped
//! build the way a `pub use` in the lib does.
//!
//! # If you are tempted to re-add it, read this first
//!
//! The name collision that made the old glob dangerous has not gone away —
//! it is just no longer this file's problem. `RetrievalHit`, `QueryResponse`,
//! `NodeKind` and `EntityMatch` on the engine are **not** the contract's
//! `RetrievalHit`, `RetrievalResponse`, `RetrievalNodeKind` and
//! `EntityMatch`. Two differences survive, checked against `tinymemory-bus`'s
//! `provider::retrieval` and tinycortex's `memory::retrieval::types`:
//!
//! - `tree_kind` is an **open string** on the contract and the closed
//!   `TreeKind` enum on the engine, so a hit crossing between them needs a
//!   conversion that can fail in one direction and is lossy in neither;
//! - the engine's field is non-optional — a bare leaf reports
//!   `TreeKind::Source` via `leaf_tree_placeholder`, where the contract
//!   encodes the same absence as `None`. Those are different answers to "which
//!   tree did this come from", and a swap would silently relabel every
//!   unsealed leaf.
//!
//! So routing an engine hit through the contract is a wire change whose
//! translation has to be written down at a handler, not a path swap.

pub mod rpc;
pub mod schemas;

/// Chunk-staging fixtures for [`rpc`]'s inline tests.
///
/// Test-only, and in a directory both memory lints skip by path — see the
/// module's own docs for why a genuinely dev-only engine reference had to move
/// out of the inline `#[cfg(test)]` block to be classified as one.
#[cfg(test)]
pub(crate) mod test_support;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_retrieval_controller_schemas,
    all_registered_controllers as all_retrieval_registered_controllers,
};
