//! Disabled-MCP facade for [`super`] (the MCP write-audit log).
//!
//! Compiled only when the `mcp` Cargo feature is OFF (see the gate in
//! [`super`]). The audit log records calls made *through the MCP server*, so
//! with MCP compiled out there is no writer and nothing to read back — the
//! disabled surface is naturally empty rather than an error.
//!
//! As in the sibling `mcp_registry` stub, [`super::types`] is ungated, so this
//! module defines no data types — only behaviour. The signatures MUST match
//! the real ones exactly; the disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift.

use anyhow::Result;

use crate::openhuman::config::Config;

use super::types::{McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};

/// Error/log text shared by every disabled-path operation. Owned locally
/// (mirroring the sibling `mcp_registry` / `mcp_server` stubs) so the module
/// stays self-contained rather than importing a private const across modules.
const DISABLED_MSG: &str = "mcp feature disabled at compile time";

// ---------------------------------------------------------------------------
// Controller registration (mirrors `schemas::all_internal_controllers`)
// ---------------------------------------------------------------------------

/// No controllers: the internal `mcp_audit` list method is unregistered, so
/// the desktop UI/CLI sees an unknown method rather than an empty history.
///
/// `src/core/all.rs` pushes this straight into its internal-controller vec
/// with no `#[cfg]` of its own — the empty vec keeps that file untouched.
pub fn all_mcp_audit_internal_controllers() -> Vec<crate::core::all::RegisteredController> {
    log::debug!("[mcp_audit] {DISABLED_MSG} — no internal controllers registered");
    Vec::new()
}

// ---------------------------------------------------------------------------
// Store surface (mirrors `store::{record_write, list_writes}`)
// ---------------------------------------------------------------------------

/// No-op that reports a synthetic row id: nothing can call an MCP write tool
/// in this build, so this is unreachable in practice. Returns `Ok` rather than
/// `Err` because the real callers treat a failure here as a logged anomaly —
/// an "audit write failed" warning would be actively misleading when the
/// audited subsystem does not exist.
pub fn record_write(_config: &Config, _record: NewMcpWriteRecord) -> Result<i64> {
    log::debug!("[mcp_audit] {DISABLED_MSG} — record_write is a no-op (no MCP writer exists)");
    Ok(0)
}

/// Empty history: no MCP writes can have occurred in this build. `Ok(vec![])`
/// (not `Err`) keeps the shape honest — the query succeeded, the log is empty.
pub fn list_writes(_config: &Config, _query: &McpWriteListQuery) -> Result<Vec<McpWriteRecord>> {
    log::debug!("[mcp_audit] {DISABLED_MSG} — list_writes returning empty history");
    Ok(Vec::new())
}
