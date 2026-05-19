//! Stdio MCP server for exposing a curated OpenHuman tool surface.
//!
//! The server is opt-in via `openhuman-core mcp` and writes only JSON-RPC
//! protocol messages to stdout. Diagnostics go through stderr logging.
//!
//! Most tools (memory tree reads, core/agent introspection) are read-only and
//! gated through `SecurityPolicy` with `ToolOperation::Read`. The one
//! exception is `agent.run_subagent`, which runs through `ToolOperation::Act`
//! and is advertised to clients via MCP tool annotations
//! (`readOnlyHint: false`, `destructiveHint: true`).

mod protocol;
mod stdio;
mod tools;

pub use stdio::run_stdio_from_cli;
pub use tools::{tool_specs, McpToolSpec};
