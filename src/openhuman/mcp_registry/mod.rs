//! MCP Registry — discover, install, and run user-chosen MCP servers.
//!
//! This is the dynamic, user-facing side of MCP-client support. It browses the
//! Smithery.ai MCP registry, persists the user's chosen installs to SQLite,
//! and (for local-spawn servers) supervises their subprocess lifecycle.
//! Installed servers' tools are surfaced to agents via the unified tool
//! registry ([`crate::openhuman::tool_registry`]).
//!
//! # Server transport model
//!
//! Today every [`InstalledServer`] is a **local subprocess** launched by npx
//! / uvx / a direct binary ([`types::CommandKind`]). The connection is stdio
//! JSON-RPC, owned by [`connections`].
//!
//! HTTP-remote MCP servers (the majority of what Smithery actually lists) are
//! **not yet modelled** as an `InstalledServer` variant — adding a remote
//! transport variant is planned follow-up work, after which the registry
//! holds both kinds.
//!
//! # Boot-time spawn
//!
//! [`boot::spawn_installed_servers`] is called from
//! `bootstrap_core_runtime` so every local-spawn server is connected as soon
//! as the core comes up. Errors are logged per-server and never block boot.
//!
//! # Relationship to `mcp_client`
//!
//! The sibling [`crate::openhuman::mcp_client`] module is the **transport
//! library** (HTTP + stdio primitives) plus the *static, config-declared*
//! server set (read from `[[mcp_client.servers]]` in TOML). Agents reach
//! that set through generic bridge tools. The static set is intentionally
//! separate from this dynamic registry — both kinds will eventually share
//! the transport primitives from `mcp_client`.
//!
//! # Modules
//! - `types`       — data structures (InstalledServer, McpTool, Smithery DTOs, …)
//! - `store`       — SQLite persistence (mcp_clients.db)
//! - `registry`    — Smithery HTTP client with 10-minute SQLite cache
//! - `connections` — global in-process connection registry (wraps
//!   [`crate::openhuman::mcp_client::McpStdioClient`] — there is no
//!   separate stdio client here)
//! - `boot`        — boot-time spawn of installed local servers
//! - `ops`         — RPC handler implementations
//! - `schemas`     — controller schemas + handler dispatch
//! - `bus`         — DomainEvent subscriber for lifecycle logging
//!
//! # Naming note
//!
//! The RPC namespace and SQLite db filename are still `mcp_clients` for
//! backwards compatibility with existing frontend code and on-disk state.
//! The Rust module path is `mcp_registry`.

pub mod boot;
pub mod bus;
pub mod connections;
mod ops;
mod registries;
mod registry;
mod schemas;
pub mod setup;
pub mod setup_ops;
pub mod store;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_mcp_registry_controller_schemas,
    all_registered_controllers as all_mcp_registry_registered_controllers,
    schemas as mcp_registry_schemas,
};

pub use types::{ConnStatus, InstalledServer, McpTool};
