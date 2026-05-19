//! MCP server for exposing a curated OpenHuman tool surface.
//!
//! Opt-in via `openhuman-core mcp` (stdio) or `openhuman-core mcp --transport http`.
//! Stdio mode writes JSON-RPC to stdout; HTTP mode speaks Streamable HTTP + SSE
//! on a local bind address. Diagnostics go through stderr logging.

mod http;
mod protocol;
mod stdio;
mod tools;

pub use http::{run_http, HttpServerConfig};
pub use stdio::run_stdio_from_cli;
pub use tools::{tool_specs, McpToolSpec};
