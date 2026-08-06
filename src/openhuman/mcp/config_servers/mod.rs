//! Static, config-declared MCP server set + stdio transport.
//!
//! [`McpServerRegistry`] holds the servers declared in the user's TOML config
//! under `[[mcp_client.servers]]`. Agents reach these through the generic
//! bridge tools in [`crate::openhuman::tools`] (`mcp_list_servers`,
//! `mcp_list_tools`, `mcp_call_tool`).
//!
//! > The TOML key stays `[[mcp_client.servers]]` and the config field stays
//! > `mcp_client` — wire/config surface is independent of module path.
//!
//! # Modules
//! - `registry`   — [`McpServerRegistry`] built from
//!   [`crate::openhuman::config::McpClientConfig`]
//! - `stdio`      — [`McpStdioClient`], subprocess JSON-RPC over stdin/stdout.
//!   All stdio transport for the *dynamic* [`super::registry`] installs flows
//!   through here; that module carries no transport code of its own.
//! - `spawn_env`  — PATH reconstruction for stdio child processes (npx/uvx
//!   resolution under GUI-stripped desktop environments)
//! - `setup_agent` — the `mcp_setup` guided-install agent
//!
//! # Compile-time gate (`mcp` feature)
//!
//! **Leaf-gated** at the `pub mod config_servers;` declaration in
//! [`super`]. There is no `stub` module and none is needed: every consumer is
//! itself `#[cfg(feature = "mcp")]`, so absence — not a disabled-error mirror
//! — is the correct off-state.
//!
//! The HTTP transport primitives live in the ungated sibling
//! [`super::http_client`]; see that module for why it is not gated.

mod registry;
pub mod setup_agent;
#[cfg(test)]
mod setup_agent_integration_test;
mod spawn_env;
mod stdio;

pub(crate) use registry::apply_safety_filter;
pub use registry::{McpRegistrySource, McpServerDefinition, McpServerRegistry, McpTransportClient};
pub use stdio::McpStdioClient;
