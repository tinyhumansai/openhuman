//! Persistent audit log for MCP write-tool calls.
//!
//! The audit table is stored in the existing memory-tree SQLite database so
//! writes and their query surface reuse the same local workspace persistence.

mod schemas;
pub mod store;
pub mod types;

pub use schemas::{
    all_controller_schemas as all_mcp_audit_controller_schemas,
    all_internal_controllers as all_mcp_audit_internal_controllers,
    all_registered_controllers as all_mcp_audit_registered_controllers,
    schemas as mcp_audit_schemas,
};
pub use store::{list_writes, record_write};
pub use types::{McpWriteListQuery, McpWriteRecord, NewMcpWriteRecord};
