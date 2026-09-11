//! The RPC surface over the write-audit log.
//!
//! The log itself moved to `tinymcp`. What is here is the `mcp_audit`
//! controller family and the types it speaks, re-exported from the wire
//! contract.
//!
//! # Where the rows are now
//!
//! The table used to be created inside this application's memory-tree database,
//! which is precisely what made it unmovable. It has its own file now,
//! `mcp_audit/mcp_audit.db`, under the same workspace directory. Rows written
//! before the move stay where they were: an audit log is history rather than
//! operational state, and nothing reads the old table any more.
//!
//! ## Compile-time gate (`mcp` feature)
//!
//! `pub mod audit;` is always compiled — it is a facade. The RPC surface is
//! gated; when the feature is off, [`stub`] mirrors the consumed surface.

#[cfg(feature = "mcp")]
mod schemas;

/// The audit payload types, from the wire contract.
pub mod types {
    pub use tinymcp_bus::{McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};
}

pub use types::{McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};

#[cfg(feature = "mcp")]
pub use schemas::{
    all_controller_schemas as all_mcp_audit_controller_schemas,
    all_internal_controllers as all_mcp_audit_internal_controllers,
    all_registered_controllers as all_mcp_audit_registered_controllers,
    schemas as mcp_audit_schemas,
};

/// Records one write against `config`'s workspace.
///
/// # Errors
///
/// Returns an error when the log cannot be opened or the row cannot be
/// written.
#[cfg(feature = "mcp")]
pub fn record_write(
    config: &crate::openhuman::config::Config,
    record: NewMcpWriteRecord,
) -> anyhow::Result<i64> {
    crate::openhuman::mcp::host::for_config(config)?
        .audit()
        .record(&record)
        .map_err(|error| anyhow::anyhow!("failed to record an mcp write: {error}"))
}

/// Lists writes recorded against `config`'s workspace.
///
/// # Errors
///
/// Returns an error when the log cannot be opened or the query fails.
#[cfg(feature = "mcp")]
pub fn list_writes(
    config: &crate::openhuman::config::Config,
    query: &McpWriteListQuery,
) -> anyhow::Result<Vec<McpWriteRecord>> {
    crate::openhuman::mcp::host::for_config(config)?
        .audit()
        .list(query)
        .map_err(|error| anyhow::anyhow!("failed to list mcp writes: {error}"))
}

// ---------------------------------------------------------------------------
// Disabled facade — compiled only when the `mcp` feature is OFF.
// ---------------------------------------------------------------------------

#[cfg(not(feature = "mcp"))]
mod stub;
#[cfg(not(feature = "mcp"))]
pub use stub::*;
