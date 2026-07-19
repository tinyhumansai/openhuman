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

//! ## Compile-time gate (`mcp` feature)
//!
//! `pub mod mcp_server;` is ALWAYS compiled — it is a facade. The protocol,
//! transports, session, and dispatch machinery are gated behind the default-ON
//! `mcp` Cargo feature; when it is off, [`stub`] mirrors the surface that
//! always-compiled callers reach (`run_stdio_from_cli`, `ensure_local_http`,
//! `LocalMcpEndpoint`, `tool_specs`) with disabled-error / empty bodies.
//!
//! `tools::types` stays UNGATED so [`McpToolSpec`] — an inert `&'static str` +
//! `Value` record consumed by the always-compiled `tool_registry` — is the
//! same real type in both builds and cannot drift.

#[cfg(feature = "mcp")]
mod http;
#[cfg(feature = "mcp")]
mod local;
#[cfg(feature = "mcp")]
mod protocol;
#[cfg(feature = "mcp")]
mod resources;
#[cfg(feature = "mcp")]
mod session;
#[cfg(feature = "mcp")]
mod stdio;
#[cfg(feature = "mcp")]
mod subagent_depth;
#[cfg(feature = "mcp")]
mod write_dispatch;

// Facade: gates its own behavioural submodules but always compiles `types`
// so `McpToolSpec` survives the gate (see the module note above).
mod tools;

#[cfg(feature = "mcp")]
pub use http::{run_http, run_http_reporting, HttpServerConfig};
#[cfg(feature = "mcp")]
pub use local::{ensure_local_http, LocalMcpEndpoint};
#[cfg(feature = "mcp")]
pub use stdio::run_stdio_from_cli;
#[cfg(feature = "mcp")]
pub use subagent_depth::{current_depth as current_subagent_depth, HEADER_SUBAGENT_DEPTH};
#[cfg(feature = "mcp")]
pub use tools::tool_specs;

// Inert tool-spec type — always compiled (see the module note above).
pub use tools::McpToolSpec;

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `mcp` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "mcp"))]
mod stub;
#[cfg(not(feature = "mcp"))]
pub use stub::*;
