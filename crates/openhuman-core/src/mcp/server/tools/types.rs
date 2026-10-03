use serde_json::Value;

/// Largest `limit` the memory tools accept; mirrors the memory domain's own cap
/// (`memory::types::MAX_LIMIT`). Oversize values are rejected, not clamped.
pub const MEMORY_MAX_LIMIT: u64 = 100;
/// Upper bound on the number of ids `memory.forget` accepts per call.
pub const MEMORY_FORGET_MAX_IDS: usize = 100;
pub const SEARXNG_SEARCH_ARGUMENTS: &[&str] = &["query", "max_results"];
pub const WEB_SEARCH_ARGUMENTS: &[&str] = &["query", "max_results", "provider"];
pub const WEB_ANSWER_ARGUMENTS: &[&str] = &["query", "depth"];
/// Upper bound for `max_results` on the search tools.
pub const SEARCH_MAX_RESULTS: usize = 20;
pub const SUBAGENT_RUN_ARGUMENTS: &[&str] = &["agent_id", "prompt"];
pub const MEMORY_RECALL_ARGUMENTS: &[&str] = &["question", "filter", "limit"];
pub const MEMORY_FETCH_ARGUMENTS: &[&str] = &["query", "mode", "filter", "limit", "cursor"];
pub const MEMORY_LIST_ARGUMENTS: &[&str] = &["filter", "limit", "cursor"];
pub const MEMORY_LEARN_ARGUMENTS: &[&str] = &["text", "kind", "confidence"];
pub const MEMORY_FORGET_ARGUMENTS: &[&str] = &["ids"];
/// Fields of a `MetaFilter` that are plain exact-match strings.
pub const FILTER_STRING_FIELDS: &[&str] = &[
    "workspace",
    "folder",
    "file_path",
    "language",
    "repo",
    "commit",
    "url",
    "thread_id",
    "agent_id",
];
pub const FILTER_FIELDS: &[&str] = &[
    "workspace",
    "folder",
    "file_path",
    "language",
    "repo",
    "commit",
    "url",
    "thread_id",
    "agent_id",
    "kinds",
    "sources",
    "tags_any",
    "observed_after",
    "observed_before",
];
pub const ITEM_KINDS: &[&str] = &["document", "conversation", "learning"];
pub const SOURCE_KINDS: &[&str] = &[
    "folder",
    "file",
    "link",
    "github",
    "rss",
    "composio",
    "conversation",
    "agent",
    "import",
];
pub const FETCH_MODES: &[&str] = &["keyword", "vector", "hybrid"];
pub const LEARNING_KINDS: &[&str] = &["preference", "fact", "procedure", "correction", "other"];

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
