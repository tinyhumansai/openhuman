//! LLM-callable wrappers for the Phase 4 memory-tree retrieval primitives
//! (issue #710). Each tool is a thin shim over one typed function in
//! [`crate::openhuman::memory::tree::retrieval`]; the `*_rpc` variants are
//! intentionally avoided because they wrap responses in an `RpcOutcome`
//! envelope that is noisier than what the LLM needs.
//!
//! All six tools share the same shape:
//! 1. Deserialize args into the matching `*Request` struct from
//!    [`crate::openhuman::memory::tree::retrieval::rpc`].
//! 2. Load the active workspace `Config` via
//!    [`crate::openhuman::config::rpc::load_config_with_timeout`].
//! 3. Call the typed retrieval function.
//! 4. Serialise the response to JSON and return it as `ToolResult::success`.
//!
//! The tools are stateless unit structs — there is no per-instance state to
//! carry, so they slot directly into `Vec<Box<dyn Tool>>` without needing
//! constructors. Logs use the `[tool]` / `[memory_tree]` prefixes per the
//! repo's debug-logging conventions.

mod drill_down;
mod fetch_leaves;
mod query_global;
mod query_source;
mod query_topic;
mod search_entities;

pub use drill_down::MemoryTreeDrillDownTool;
pub use fetch_leaves::MemoryTreeFetchLeavesTool;
pub use query_global::MemoryTreeQueryGlobalTool;
pub use query_source::MemoryTreeQuerySourceTool;
pub use query_topic::MemoryTreeQueryTopicTool;
pub use search_entities::MemoryTreeSearchEntitiesTool;
