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

//! ## Compile-time gate (`mcp` feature)
//!
//! `pub mod mcp_registry;` is ALWAYS compiled — it is a facade. Everything
//! that carries behaviour (Smithery HTTP client, SQLite store, live
//! connection map, boot spawn, supervisor, OAuth, RPC surface) is gated
//! behind the default-ON `mcp` Cargo feature; when the feature is off,
//! [`stub`] mirrors the subset of the surface that always-compiled callers
//! depend on with no-op / `Err` / empty-`Vec` bodies, so those callers need no
//! `#[cfg]` of their own.
//!
//! [`types`] stays UNGATED on purpose: it is inert serde data (`serde` +
//! `serde_json` only) consumed by the always-compiled orchestrator prompt
//! builder. Sharing the one real definition across both builds means the
//! disabled build cannot drift from the enabled one — the stub carries
//! behaviour, never duplicated types.

#[cfg(feature = "mcp")]
pub mod boot;
#[cfg(feature = "mcp")]
pub mod bus;
#[cfg(feature = "mcp")]
pub mod connections;
#[cfg(feature = "mcp")]
mod curation;
#[cfg(feature = "mcp")]
pub mod oauth;
#[cfg(feature = "mcp")]
pub mod ops;
#[cfg(feature = "mcp")]
mod registries;
#[cfg(feature = "mcp")]
mod registry;
#[cfg(feature = "mcp")]
mod schemas;
#[cfg(feature = "mcp")]
pub mod setup;
#[cfg(feature = "mcp")]
pub mod setup_ops;
#[cfg(feature = "mcp")]
pub mod store;
#[cfg(feature = "mcp")]
pub mod supervisor;
#[cfg(feature = "mcp")]
pub mod tools;

// Inert serde types — always compiled (see the module note above).
pub mod types;

#[cfg(feature = "mcp")]
pub use schemas::{
    all_controller_schemas as all_mcp_registry_controller_schemas,
    all_registered_controllers as all_mcp_registry_registered_controllers,
    schemas as mcp_registry_schemas,
};

pub use types::{ConnStatus, InstalledServer, McpTool};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `mcp` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "mcp"))]
mod stub;
#[cfg(not(feature = "mcp"))]
pub use stub::*;
