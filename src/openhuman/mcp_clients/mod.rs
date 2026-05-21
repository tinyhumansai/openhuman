//! MCP Clients domain — browse the Smithery.ai MCP registry, install servers
//! locally, spawn them as stdio subprocesses, and expose their tools to agents.
//!
//! # Modules
//! - `types`       — data structures (InstalledServer, McpTool, Smithery DTOs, …)
//! - `store`       — SQLite persistence (mcp_clients.db)
//! - `registry`    — Smithery HTTP client with 10-minute SQLite cache
//! - `client`      — MCP stdio JSON-RPC client + FakeMcpTransport test double
//! - `connections` — global in-process connection registry
//! - `ops`         — RPC handler implementations
//! - `schemas`     — controller schemas + handler dispatch
//! - `bus`         — DomainEvent subscriber for lifecycle logging

pub mod bus;
mod client;
pub(crate) mod connections;
mod ops;
mod registry;
mod schemas;
mod store;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_mcp_clients_controller_schemas,
    all_registered_controllers as all_mcp_clients_registered_controllers,
    schemas as mcp_clients_schemas,
};

pub use types::{ConnStatus, InstalledServer, McpTool};
