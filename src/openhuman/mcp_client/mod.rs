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
//! - `client`   — [`McpHttpClient`] and shared MCP protocol types
//! - `stdio`    — [`McpStdioClient`]
//! - `registry` — [`McpServerRegistry`] built from
//!   [`crate::openhuman::config::McpClientConfig`]

mod client;
mod registry;
mod stdio;

pub use client::{
    redact_endpoint, AuthorizationServerMetadata, McpAuthChallenge, McpAuthorizationContext,
    McpHttpClient, McpInitializeResult, McpRemoteTool, McpServerToolResult, McpSseEvent,
    ProtectedResourceMetadata,
};
pub use registry::{McpRegistrySource, McpServerDefinition, McpServerRegistry, McpTransportClient};
pub use stdio::McpStdioClient;
