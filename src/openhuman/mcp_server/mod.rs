//! Stdio MCP server for exposing a curated, read-only OpenHuman tool surface.
//!
//! The server is opt-in via `openhuman-core mcp` and writes only JSON-RPC
//! protocol messages to stdout. Diagnostics go through stderr logging.

mod protocol;
mod stdio;
mod tools;

pub use stdio::run_stdio_from_cli;
