//! Types for the MCP write audit log.

use serde::{Deserialize, Serialize};

/// One row of the `mcp_writes` audit table.
///
/// Recorded per successful **and** failed MCP write tool invocation —
/// both signals matter:
///
/// - **Success rows** answer "what did Claude Desktop write into my
///   memory this week?" for accountability / compliance.
/// - **Failure rows** are the abuse-detection signal: a misbehaving
///   client repeatedly hitting the write surface but bouncing off the
///   policy gate or `doc_put` validation will show up as a burst of
///   `success = false` rows.
///
/// `args_summary` deliberately stores **identifying metadata only**
/// (not the document content itself) — the content lives in the
/// memory tree via `doc_put`, so duplicating it here would bloat the
/// audit table without adding information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpWriteRecord {
    /// Wall-clock UNIX time in milliseconds.
    pub timestamp_ms: i64,
    /// Provenance string captured from `McpSession::source_type()` —
    /// `"mcp:claude-desktop"` / `"mcp:cursor"` / fallback `"mcp"` when
    /// the MCP client didn't supply `clientInfo.name` during
    /// `initialize`. Indexed for fast per-client queries.
    pub client_source_type: String,
    /// MCP tool name that produced the write
    /// (`"memory.store"` / `"memory.note"` / `"tree.tag"`). Indexed
    /// for "show me all my tag writes" queries.
    pub tool_name: String,
    /// Slim JSON object capturing identifying args without duplicating
    /// document content. Shape varies per tool — see #2536 body for
    /// the per-tool schema. `None` when args summarisation produced
    /// no recordable fields (treated as a non-fatal soft-failure).
    pub args_summary: Option<String>,
    /// `document_id` returned by `memory_doc_put` on the success path.
    /// `None` on failure rows or when the upstream RPC reply didn't
    /// include a document id.
    pub resulting_chunk_id: Option<String>,
    /// `true` when the underlying RPC returned `Ok`; `false` when it
    /// returned `Err` or the RPC was not registered.
    pub success: bool,
    /// Populated only when `success == false` — the upstream error
    /// message at the `dispatch_write_tool` boundary. Truncated to
    /// 1 KiB at insert time so a runaway error stack doesn't bloat
    /// the table.
    pub error_message: Option<String>,
}
