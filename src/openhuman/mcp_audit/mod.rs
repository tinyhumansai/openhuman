//! Persistent audit log for MCP write-tool calls.
//!
//! The audit table is stored in the existing memory-tree SQLite database so
//! writes and their query surface reuse the same local workspace persistence.

//! ## Compile-time gate (`mcp` feature)
//!
//! `pub mod mcp_audit;` is ALWAYS compiled — it is a facade. The SQLite store
//! and RPC surface are gated behind the default-ON `mcp` Cargo feature; when
//! it is off, [`stub`] mirrors the consumed surface with no-op / empty bodies.
//!
//! [`types`] stays UNGATED: it is inert serde data (`serde` + `serde_json`
//! only), so both builds share the one real definition and cannot drift.

#[cfg(feature = "mcp")]
mod schemas;
#[cfg(feature = "mcp")]
pub mod store;

// Inert serde types — always compiled (see the module note above).
pub mod types;

#[cfg(feature = "mcp")]
pub use schemas::{
    all_controller_schemas as all_mcp_audit_controller_schemas,
    all_internal_controllers as all_mcp_audit_internal_controllers,
    all_registered_controllers as all_mcp_audit_registered_controllers,
    schemas as mcp_audit_schemas,
};
#[cfg(feature = "mcp")]
pub use store::{list_writes, record_write};
pub use types::{McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `mcp` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "mcp"))]
mod stub;
#[cfg(not(feature = "mcp"))]
pub use stub::*;
