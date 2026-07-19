//! MCP tool catalog, parameter validation, and dispatch logic.
//!
//! Split into focused sub-modules:
//!   - `types`    — `McpToolSpec`, `ToolCallError`, shared constants
//!   - `specs`    — tool spec builders and schema helpers
//!   - `params`   — argument parsing and RPC param construction
//!   - `dispatch` — `call_tool`, `list_tools_result`, agent/subagent handlers

//! ## Compile-time gate (`mcp` feature)
//!
//! `types` is ALWAYS compiled: [`McpToolSpec`] is inert data (`&'static str` +
//! `Value`, no deps beyond `serde_json`) that the always-compiled
//! `tool_registry` names. The behavioural siblings — which reach into the RPC
//! surface, security policy, and every gated domain — are gated.

#[cfg(feature = "mcp")]
mod dispatch;
#[cfg(feature = "mcp")]
mod params;
#[cfg(feature = "mcp")]
mod specs;

// Inert type module — always compiled (see the module note above).
mod types;

// Public API consumed by the rest of `mcp_server`
#[cfg(feature = "mcp")]
pub use dispatch::{call_tool, list_tools_result, tool_error, tool_success};
#[cfg(feature = "mcp")]
pub use specs::tool_specs;
#[cfg(feature = "mcp")]
pub use types::ToolCallError;

pub use types::McpToolSpec;

// Re-exports needed by the companion test module via `use super::*`.
// Guarded by `#[cfg(test)]` so they do not pollute the production namespace.
#[cfg(all(test, feature = "mcp"))]
pub use crate::core::all;
#[cfg(all(test, feature = "mcp"))]
pub use crate::openhuman::config::rpc as config_rpc;
#[cfg(all(test, feature = "mcp"))]
pub use crate::openhuman::tools::SEARXNG_MAX_RESULTS;
#[cfg(all(test, feature = "mcp"))]
pub use params::{build_rpc_params, slug_from};
#[cfg(all(test, feature = "mcp"))]
pub use serde_json::{json, Value};
#[cfg(all(test, feature = "mcp"))]
pub use types::{DEFAULT_LIMIT, MAX_LIMIT, TREE_TAG_MAX_TAGS, TREE_TAG_MAX_TAG_LENGTH};

#[cfg(all(test, feature = "mcp"))]
#[path = "../tools_tests.rs"]
mod tests;
