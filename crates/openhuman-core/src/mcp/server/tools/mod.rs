//! MCP tool catalog, parameter validation, and dispatch logic.
//!
//! Split into focused sub-modules:
//!   - `types`    — `McpToolSpec` and the argument/limit constants
//!   - `specs`    — tool spec builders, schema helpers, and the conversion to
//!     `tinymcp_bus::ServerToolSpec`
//!   - `params`   — OpenHuman's per-tool argument policy and RPC param
//!     construction, with generic MCP argument validation delegated to the
//!     configured TinyMCP module
//!   - `dispatch` — `call_tool`, `list_tool_specs`, agent/subagent handlers
//!
//! `ToolCallError` is part of `tinymcp_bus` and shared with the module.

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

// Public API consumed by the rest of `mcp::server`
#[cfg(feature = "mcp")]
pub use dispatch::{call_tool, list_tool_specs, tool_error, tool_success};
#[cfg(feature = "mcp")]
pub use specs::{server_tool_spec, tool_specs};
#[cfg(feature = "mcp")]
pub use tinymcp_bus::ToolCallError;

pub use types::McpToolSpec;

// Re-exports needed by the companion test module via `use super::*`.
// Guarded by `#[cfg(test)]` so they do not pollute the production namespace.
#[cfg(all(test, feature = "mcp"))]
pub use crate::config::rpc as config_rpc;
#[cfg(all(test, feature = "mcp"))]
pub use crate::core::all;
#[cfg(all(test, feature = "mcp"))]
pub(crate) use params::build_rpc_params_for_test;
#[cfg(all(test, feature = "mcp"))]
pub use serde_json::{json, Value};
#[cfg(all(test, feature = "mcp"))]
pub use types::{MEMORY_FORGET_MAX_IDS, MEMORY_MAX_LIMIT};

#[cfg(all(test, feature = "mcp"))]
#[path = "../tools_tests.rs"]
mod tests;
