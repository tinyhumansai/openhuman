//! MCP server for exposing a curated OpenHuman tool surface.
//!
//! Opt-in via `openhuman-core mcp` (stdio) or `openhuman-core mcp --transport http`.
//! Stdio mode writes newline-delimited JSON-RPC to stdout; HTTP mode speaks
//! Streamable HTTP + SSE on a local bind address. Diagnostics go through stderr logging.
//!
//! Most tools (memory tree reads, core/agent introspection) are read-only and
//! gated through `SecurityPolicy` with `ToolOperation::Read`. The one
//! exception is `agent.run_subagent`, which runs through `ToolOperation::Act`
//! and is advertised to clients via MCP tool annotations
//! (`readOnlyHint: false`, `destructiveHint: true`).

mod http;
mod protocol;
mod resources;
mod session;
mod stdio;
mod tools;
mod write_dispatch;

pub use http::{run_http, HttpServerConfig};
pub use stdio::run_stdio_from_cli;
pub use tools::{tool_specs, McpToolSpec};
