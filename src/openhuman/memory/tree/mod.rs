//! Host layer over what used to be an embedded memory tree.
//!
//! The domain itself lives in the extracted engine crates; what stays here is
//! its JSON-RPC surface — handlers and controller schemas name OpenHuman's
//! `RpcOutcome` and `ControllerSchema`, which the engine crate cannot see.
//!
//! # The `pub use tinymemory_core::tree::*` shim is gone (#5560)
//!
//! This file carried a glob re-export so that every historical
//! `memory::tree::…` path kept resolving. Measured by deleting the line and
//! reading the compiler, its production surface was **empty**: `score` is
//! `MemoryChunks::chunk_score` and the contract's own `DEFAULT_DROP_THRESHOLD`,
//! `summarise` is `MemoryTree::summarise` with the contract's owned DTOs, the
//! seven `Tree{LabelStrategy, LeafPayload, ReadHit, ReadRequest, ReadResult,
//! WriteOutcome, WriteRequest}` I/O types went unnamed in `src/`, and `nlp` and
//! `graph` had no caller under this path at all. What the glob still served
//! was tests — this crate's `*_tests.rs`, the recap's `#[cfg(test)]` arm and
//! `tests/raw_coverage/` reaching `score`, `summarise` and `ingest`.
//!
//! A production `pub use` that exists only to serve a test is exactly what
//! keeps a crate in `[dependencies]`, which is the thing #5560 is removing. So
//! those tests name the engine crates directly now —
//! `tinymemory_core::tree::{score, summarise, ingest}` and
//! `tinycortex::memory::tree::Tree*`, served by the `[dev-dependencies]`
//! `tinymemory-core` entry the way the earlier-repointed `tests/` targets
//! already were — and the glob is deleted. What resolves under
//! `memory::tree::…` is the host's own surface: the four `pub mod`s and the
//! controller registries below.

pub mod health;
pub mod retrieval;
// `tree::tree` mirrors `tinymemory_core::tree::tree` — the wrapper has to keep
// the extracted crate's path shape so every historical `memory::tree::tree::…`
// reference still resolves. Renaming it here would break that for a lint.
#[allow(clippy::module_inception)]
pub mod tree;
pub mod tree_runtime;

// Controller registries. These aggregate the RPC surface that stayed here, so
// they cannot live in the extracted crate alongside the rest of `tree`.
pub use crate::openhuman::memory::schema::{
    all_controller_schemas as all_memory_tree_controller_schemas,
    all_registered_controllers as all_memory_tree_registered_controllers,
};
pub use retrieval::{all_retrieval_controller_schemas, all_retrieval_registered_controllers};
pub use tree_runtime::{
    all_tree_summarizer_controller_schemas, all_tree_summarizer_registered_controllers,
};
