//! MCP tool catalog, parameter validation, and dispatch logic.
//!
//! Split into focused sub-modules:
//!   - `types`    — `McpToolSpec`, `ToolCallError`, shared constants
//!   - `specs`    — tool spec builders and schema helpers
//!   - `params`   — argument parsing and RPC param construction
//!   - `dispatch` — `call_tool`, `list_tools_result`, agent/subagent handlers

mod dispatch;
mod params;
mod specs;
mod types;

// Public API consumed by the rest of `mcp_server`
pub use dispatch::{call_tool, list_tools_result, tool_error, tool_success};
pub use specs::{
    base_tool_specs, list_tools_result_for_config, list_tools_result_from_specs, searxng_tool_spec,
    tool_specs,
};
pub use types::{McpToolSpec, ToolCallError};

// Re-exports needed by the companion test module via `use super::*`.
// Guarded by `#[cfg(test)]` so they do not pollute the production namespace.
#[cfg(test)]
pub use crate::core::all;
#[cfg(test)]
pub use crate::openhuman::config::rpc as config_rpc;
#[cfg(test)]
pub use crate::openhuman::tools::SEARXNG_MAX_RESULTS;
#[cfg(test)]
pub use params::{build_rpc_params, slug_from};
#[cfg(test)]
pub use serde_json::{json, Value};
#[cfg(test)]
pub use types::{DEFAULT_LIMIT, MAX_LIMIT, TREE_TAG_MAX_TAGS, TREE_TAG_MAX_TAG_LENGTH};

#[cfg(test)]
#[path = "../tools_tests.rs"]
mod tests;
