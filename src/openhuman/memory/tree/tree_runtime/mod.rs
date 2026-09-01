//! The markdown time tree's host surface — JSON-RPC handlers, controller
//! schemas, the `tree-summarizer` CLI, and the event subscriber.
//!
//! # The glob is gone (#5560)
//!
//! This module was a host layer *over* `tinymemory_core::tree::tree_runtime`,
//! and `pub use tinymemory_core::tree::tree_runtime::*;` was the last
//! production re-export of the engine crate in this tree. It supplied two
//! modules — `engine` (`run_summarization` / `rebuild_tree` /
//! `run_hourly_loop`) and `store` (~18 `Config`-taking wrappers over
//! `tinycortex::memory::tree::runtime::store`) — and both were pinned by one
//! call, `engine_config(config)`, because nothing host-side could build a
//! `tinycortex::memory::MemoryConfig`.
//!
//! That is no longer the blocker it was, because the host no longer needs to
//! build one: the tree is reached through the **contract's** six runtime doors
//! (`runtime_buffer_write`, `runtime_read_node`, `runtime_read_children`,
//! `runtime_tree_status`, `runtime_summarize`, `runtime_rebuild`), and the
//! driver that serves them builds the engine config on its own side of the bus,
//! through its own embedder ladder. `ops.rs` carries the per-member mapping.
//!
//! `memory::tools::flavour` was blocked on the same call and took the same
//! route out, through `MemoryTree::flavour_profile`.
//!
//! What is left here is what could only ever have lived host-side: the handlers
//! and schemas name OpenHuman's `RpcOutcome` and `ControllerSchema`, the CLI
//! names its argument parsing, and the subscriber names `DomainEvent`.

// The summary-tree node model is **contract** vocabulary, not engine
// vocabulary: it is defined in `tinymemory-bus`, and the engine crate — while
// it was still re-exported here — merely re-exported the same items, so these
// names were never a second set of types. They are now the only set, and the
// call sites on `memory::tree::tree_runtime::estimate_tokens` and friends
// needed no edit when the glob went, which is what naming the contract
// explicitly bought.
pub use crate::openhuman::memory::api::tree as types;
pub use crate::openhuman::memory::api::tree::{
    derive_node_ids, derive_parent_id, estimate_tokens, level_from_node_id, node_id_to_path,
    IngestRequest, NodeLevel, QueryResult, TreeNode, TreeStatus,
};

pub mod ops;
pub mod schemas;

/// The driver the handler and CLI tests bind, now that both resolve one.
#[cfg(test)]
pub(crate) mod test_support;

pub use ops as rpc;

pub mod bus;

/// The `openhuman memory tree` CLI subcommands, which drive the RPC handlers.
pub mod cli;

// The controller aggregators this domain's RPC surface defines. Aliased
// exactly as the pre-extraction module exported them.
pub use schemas::{
    all_controller_schemas as all_tree_summarizer_controller_schemas,
    all_registered_controllers as all_tree_summarizer_registered_controllers,
};
