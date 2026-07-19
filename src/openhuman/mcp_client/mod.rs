//! MCP client transport library + static-config server set.
//!
//! Two responsibilities:
//!
//! 1. **Transport primitives** — [`McpHttpClient`] (Streamable HTTP + OAuth +
//!    SSE per the MCP spec) and [`McpStdioClient`] (subprocess JSON-RPC over
//!    stdin/stdout). These types are reusable building blocks for any module
//!    that needs to *talk to* a remote MCP server.
//!
//! 2. **Static server set** — [`McpServerRegistry`] holds servers declared in
//!    the user's TOML config under `[[mcp_client.servers]]`. Agents reach
//!    these via the generic bridge tools in
//!    [`crate::openhuman::tools::impl::network::mcp`] (`mcp_list_servers`,
//!    `mcp_list_tools`, `mcp_call_tool`). The bespoke `gitbooks` tool also
//!    consumes [`McpHttpClient`] directly.
//!
//! # Relationship to `mcp_registry`
//!
//! The sibling [`crate::openhuman::mcp_registry`] module owns the *dynamic*,
//! user-installed Smithery / official-registry MCP servers (full RPC CRUD,
//! SQLite persistence, live connection registry, boot-time spawn). All stdio
//! transport for those installs flows through this module's
//! [`McpStdioClient`] — `mcp_registry` carries no transport code of its own.
//!
//! In short:
//! - **`mcp_client`** (this module): transport library + read-only static
//!   server set declared in user config.
//! - **`mcp_registry`** (sibling): dynamic Smithery installations, lifecycle,
//!   persistence, and RPC surface.
//!
//! # Modules
//! - `client`    — [`McpHttpClient`] and shared MCP protocol types
//! - `stdio`     — [`McpStdioClient`]
//! - `spawn_env` — PATH reconstruction for stdio child processes (npx/uvx
//!   resolution under GUI-stripped desktop environments)
//! - `registry` — [`McpServerRegistry`] built from
//!   [`crate::openhuman::config::McpClientConfig`]

//! ## Compile-time gate (`mcp` feature) — SPLIT facade
//!
//! Unlike the sibling MCP modules, this one is NOT gated wholesale, because
//! the `mcp_client` directory does not match the real dependency graph. Two of
//! its submodules are **mis-housed shared utilities** with live consumers that
//! have nothing to do with the MCP subsystem, so they stay ALWAYS COMPILED:
//!
//! * [`sanitize`] — the orchestrator prompt builder runs *skill* descriptions
//!   through `sanitize::sanitize_for_llm`. Nothing to do with MCP; stubbing it
//!   would silently corrupt the orchestrator prompt in slim builds.
//! * [`client`] — the bespoke `gitbooks` docs tool dials [`McpHttpClient`] +
//!   [`redact_endpoint`] directly (GitBook is modelled as a legacy MCP server).
//!   Stubbing it would break a docs tool that users reach in slim builds.
//!   Keeping it compiled also keeps [`McpUnauthorizedError`] — and therefore
//!   the `McpServerNeedsAuth` classifier coupling test in
//!   `core::observability` — always compiled, with no `#[cfg]` and no
//!   wording-drift leak.
//!
//! Gated behind the default-ON `mcp` feature: [`registry`] (the static,
//! config-declared server set), `stdio`, `spawn_env`, and `setup_agent` — the
//! parts that genuinely constitute MCP-subsystem behaviour.
//!
//! In short: **the gate follows the real dependency graph, not the directory
//! name.** Relocating `sanitize` + `client` out of `mcp_client` is worthwhile
//! follow-up, but is deliberately out of scope here.
//!
//! There is no `stub` module: every gated item's consumers are themselves
//! gated, so nothing needs a disabled mirror.

mod client;
#[cfg(feature = "mcp")]
mod registry;
pub mod sanitize;
#[cfg(feature = "mcp")]
pub mod setup_agent;
#[cfg(all(test, feature = "mcp"))]
mod setup_agent_integration_test;
#[cfg(feature = "mcp")]
mod spawn_env;
#[cfg(feature = "mcp")]
mod stdio;

pub use client::{
    redact_endpoint, AuthorizationServerMetadata, McpAuthChallenge, McpAuthorizationContext,
    McpHttpClient, McpInitializeResult, McpRemoteTool, McpServerToolResult, McpSseEvent,
    McpUnauthorizedError, ProtectedResourceMetadata,
};
#[cfg(feature = "mcp")]
pub(crate) use registry::apply_safety_filter;
#[cfg(feature = "mcp")]
pub use registry::{McpRegistrySource, McpServerDefinition, McpServerRegistry, McpTransportClient};
#[cfg(feature = "mcp")]
pub use stdio::McpStdioClient;
