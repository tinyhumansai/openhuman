use serde_json::Value;

pub const DEFAULT_LIMIT: u64 = 10;
pub const MAX_LIMIT: u64 = 50;
pub const QUERY_ARGUMENTS: &[&str] = &["query", "k"];
pub const SEARXNG_SEARCH_ARGUMENTS: &[&str] = &["query", "categories", "language", "max_results"];
pub const TREE_READ_CHUNK_ARGUMENTS: &[&str] = &["chunk_id"];
pub const SUBAGENT_RUN_ARGUMENTS: &[&str] = &["agent_id", "prompt"];
pub const TREE_BROWSE_ARGUMENTS: &[&str] = &[
    "source_kinds",
    "source_ids",
    "entity_ids",
    "since_ms",
    "until_ms",
    "query",
    "k",
    "offset",
];
pub const TREE_TOP_ENTITIES_ARGUMENTS: &[&str] = &["kind", "k"];
pub const TREE_LIST_SOURCES_ARGUMENTS: &[&str] = &["user_email_hint"];
pub const MEMORY_STORE_ARGUMENTS: &[&str] = &["title", "content", "namespace", "tags"];
pub const MEMORY_NOTE_ARGUMENTS: &[&str] = &["chunk_id", "note_text"];
pub const TREE_TAG_ARGUMENTS: &[&str] = &["chunk_id", "tags"];
/// Upper bound on the number of tags `tree.tag` accepts per call.
/// Matches the "explicit rejection over silent clamping" pattern used
/// elsewhere in the MCP layer; prevents a misbehaving client from
/// flooding a chunk's tag-record document with thousands of entries.
pub const TREE_TAG_MAX_TAGS: usize = 50;
/// Upper bound on a single tag's character length. Tags are categorical
/// labels — anything past ~128 chars is almost certainly free-form text
/// that should be `memory.note` instead, so reject up-front to surface
/// the misuse rather than silently writing a giant token into the
/// queryable `tags` index.
pub const TREE_TAG_MAX_TAG_LENGTH: usize = 128;

#[derive(Debug, Clone)]
pub struct McpToolSpec {
    pub name: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub rpc_method: Option<&'static str>,
    pub input_schema: Value,
    /// MCP `ToolAnnotations` per the 2025-03-26+ spec — `readOnlyHint`,
    /// `destructiveHint`, `idempotentHint`, `openWorldHint`. Hints, not
    /// guarantees; clients use them to surface accurate safety affordances
    /// (e.g. Claude Desktop's "this tool can take destructive actions"
    /// confirmation gate). Per spec, destructive/idempotent are meaningful
    /// only when `readOnlyHint == false`, so read-only tools omit them.
    pub annotations: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolCallError {
    /// Client-side problem: malformed arguments, unknown tool, validation
    /// failure. Maps to JSON-RPC `-32602 Invalid params`.
    InvalidParams(String),
    /// Server-side problem outside the caller's control: config load failure,
    /// missing platform resources. Maps to JSON-RPC `-32603 Internal error`.
    /// Kept distinct from `InvalidParams` so MCP clients don't display
    /// internal failures as if the user supplied bad arguments.
    Internal(String),
}

impl ToolCallError {
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidParams(message) | Self::Internal(message) => message,
        }
    }

    /// JSON-RPC error code corresponding to this variant.
    pub fn code(&self) -> i64 {
        match self {
            Self::InvalidParams(_) => -32602,
            Self::Internal(_) => -32603,
        }
    }

    /// JSON-RPC error `message` field (short, spec-canonical phrase). The
    /// human-readable detail belongs in the response's `data` field.
    pub fn jsonrpc_message(&self) -> &'static str {
        match self {
            Self::InvalidParams(_) => "Invalid params",
            Self::Internal(_) => "Internal error",
        }
    }
}
