//! Persistent audit log for MCP write tool invocations
//! (`memory.store`, `memory.note`, `tree.tag`).
//!
//! Closes out Q4 of the Phase 3 RFC (#2269) — replaces the
//! `tracing::info!`-only audit trail with a queryable SQLite-backed
//! `mcp_writes` table colocated in the existing memory-tree DB.
//!
//! Issue: #2536.
//!
//! ## V1 scope (this module)
//!
//! - Schema (`mcp_writes`) and migration (additive `CREATE TABLE IF NOT EXISTS`
//!   in `tree::store::SCHEMA`).
//! - `McpWriteRecord` struct + `record_write` insert helper.
//! - Best-effort coupling to the MCP write dispatch path
//!   (`mcp_server::tools::dispatch_write_tool`) — audit insert failures are
//!   logged but never abort the underlying write (Q2=A in #2536).
//!
//! ## Out of scope (follow-ups once Q4 is ratified)
//!
//! - Query RPC (`openhuman.mcp_audit_list`).
//! - MCP-client-side exposure of the audit log.
//! - UI surface for browsing the audit history.
//! - Retention / pruning policy.

mod store;
#[cfg(test)]
mod tests;
mod types;

pub use store::record_write;
pub use types::McpWriteRecord;
