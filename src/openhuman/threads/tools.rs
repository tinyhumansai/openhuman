//! LLM-callable wrappers over the `threads` domain (conversation threads).
//!
//! These tools let the agent enumerate, create, retitle, relabel, and read
//! conversation threads and their messages, inspect/clear in-flight turn
//! state, and read/write a thread's kanban task board. Most tools deserialize
//! their args into the same request struct the RPC layer uses and delegate to
//! [`crate::openhuman::threads::ops`] (which returns
//! `RpcOutcome<ApiEnvelope<_>>`); the wrapper emits the inner envelope as JSON.
//!
//! Read/bounded-write tools are default-enabled. The destructive ones —
//! `thread_delete` (one thread) and `thread_purge_all` (every thread) — ship
//! default-OFF via `tools/user_filter.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use serde_json::json;

use crate::openhuman::agent::task_board::{
    board_for_thread, TaskBoard, TaskBoardCard, TaskBoardStore,
};
use crate::openhuman::config::Config;
use crate::openhuman::memory::{
    AppendConversationMessageRequest, ConversationMessagesRequest, CreateConversationThreadRequest,
    DeleteConversationThreadRequest, EmptyRequest, GenerateConversationThreadTitleRequest,
    UpdateConversationMessageRequest, UpdateConversationThreadLabelsRequest,
    UpdateConversationThreadTitleRequest,
};
use crate::openhuman::threads::ops;
use crate::openhuman::threads::turn_state::{ClearTurnStateRequest, GetTurnStateRequest};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

fn read_required_str(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    args.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing required string argument `{key}`"))
}

/// Deserialize tool args into a request struct with a uniform error.
fn parse_req<T: serde::de::DeserializeOwned>(
    args: serde_json::Value,
    tool: &str,
) -> anyhow::Result<T> {
    serde_json::from_value(args).map_err(|e| anyhow::anyhow!("{tool}: invalid arguments: {e}"))
}

/// Emit a thread ops `RpcOutcome` envelope as a JSON tool result.
macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// List conversation threads.
pub struct ThreadListTool;

#[async_trait]
impl Tool for ThreadListTool {
    fn name(&self) -> &str {
        "thread_list"
    }

    fn description(&self) -> &str {
        "List conversation threads (id, title, labels, timestamps). Use to find \
         a thread id before reading its messages, retitling, or relabeling."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] list invoked");
        emit!(ops::threads_list(EmptyRequest {}).await, "thread_list")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Read one thread's metadata (filtered from the list).
pub struct ThreadReadTool;

/// Recursively find the first JSON object whose id field matches `target`.
fn find_thread_obj(value: &serde_json::Value, target: &str) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => {
            for key in ["id", "thread_id", "threadId"] {
                if map.get(key).and_then(|v| v.as_str()) == Some(target) {
                    return Some(value.clone());
                }
            }
            map.values().find_map(|v| find_thread_obj(v, target))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_thread_obj(v, target)),
        _ => None,
    }
}

#[async_trait]
impl Tool for ThreadReadTool {
    fn name(&self) -> &str {
        "thread_read"
    }

    fn description(&self) -> &str {
        "Read one conversation thread's metadata by `thread_id` (title, labels, \
         timestamps). For the messages themselves, use `thread_message_list`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "thread_id": { "type": "string", "description": "Thread id." } },
            "required": ["thread_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] read invoked");
        let thread_id = read_required_str(&args, "thread_id")?;
        let outcome = ops::threads_list(EmptyRequest {})
            .await
            .map_err(|e| anyhow::anyhow!("thread_read: {e}"))?;
        let value = serde_json::to_value(&outcome.value)?;
        match find_thread_obj(&value, &thread_id) {
            Some(found) => Ok(ToolResult::success(serde_json::to_string(&found)?)),
            None => Err(anyhow::anyhow!(
                "thread_read: thread `{thread_id}` not found"
            )),
        }
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Create a new thread.
pub struct ThreadCreateTool;

#[async_trait]
impl Tool for ThreadCreateTool {
    fn name(&self) -> &str {
        "thread_create"
    }

    fn description(&self) -> &str {
        "Create a new conversation thread, optionally with `labels` and a \
         `personality_id`. Returns the new thread summary (including its id)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "labels": { "type": "array", "items": { "type": "string" } },
                "personality_id": { "type": "string" }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] create invoked");
        let req: CreateConversationThreadRequest = parse_req(args, "thread_create")?;
        emit!(ops::thread_create_new(req).await, "thread_create")
    }
}

/// Update a thread's title.
pub struct ThreadUpdateTitleTool;

#[async_trait]
impl Tool for ThreadUpdateTitleTool {
    fn name(&self) -> &str {
        "thread_update_title"
    }

    fn description(&self) -> &str {
        "Set a conversation thread's title. Requires `thread_id` and a \
         non-empty `title`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "title": { "type": "string" }
            },
            "required": ["thread_id", "title"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] update_title invoked");
        let req: UpdateConversationThreadTitleRequest = parse_req(args, "thread_update_title")?;
        emit!(ops::thread_update_title(req).await, "thread_update_title")
    }
}

/// Update a thread's labels.
pub struct ThreadUpdateLabelsTool;

#[async_trait]
impl Tool for ThreadUpdateLabelsTool {
    fn name(&self) -> &str {
        "thread_update_labels"
    }

    fn description(&self) -> &str {
        "Replace a conversation thread's labels with the supplied `labels` \
         array (empty clears all labels). Requires `thread_id`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "labels": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["thread_id", "labels"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] update_labels invoked");
        let req: UpdateConversationThreadLabelsRequest = parse_req(args, "thread_update_labels")?;
        emit!(ops::thread_update_labels(req).await, "thread_update_labels")
    }
}

/// List a thread's messages.
pub struct ThreadMessageListTool;

#[async_trait]
impl Tool for ThreadMessageListTool {
    fn name(&self) -> &str {
        "thread_message_list"
    }

    fn description(&self) -> &str {
        "List the messages in a conversation thread by `thread_id`, in order."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "thread_id": { "type": "string" } },
            "required": ["thread_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] message_list invoked");
        let req: ConversationMessagesRequest = parse_req(args, "thread_message_list")?;
        emit!(ops::messages_list(req).await, "thread_message_list")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Search past conversations (all threads) for messages matching a query.
pub struct ThreadTranscriptSearchTool;

impl ThreadTranscriptSearchTool {
    /// Default and ceiling for the number of matches returned. Kept small so a
    /// context-gathering caller's prompt only grows by a bounded amount.
    const DEFAULT_LIMIT: usize = 8;
    const MAX_LIMIT: usize = 25;
    /// Per-hit snippet cap (chars) — enough to recognise the message without
    /// dumping whole turns into the caller's context.
    const SNIPPET_CHARS: usize = 220;

    fn snippet(content: &str) -> String {
        let collapsed: String = content.split_whitespace().collect::<Vec<_>>().join(" ");
        if collapsed.chars().count() <= Self::SNIPPET_CHARS {
            return collapsed;
        }
        let cut = collapsed
            .char_indices()
            .nth(Self::SNIPPET_CHARS)
            .map(|(i, _)| i)
            .unwrap_or(collapsed.len());
        format!("{}…", &collapsed[..cut])
    }
}

#[async_trait]
impl Tool for ThreadTranscriptSearchTool {
    fn name(&self) -> &str {
        "transcript_search"
    }

    fn description(&self) -> &str {
        "Search your PAST conversations across all threads for messages matching \
         a query (keyword/substring, recency-ranked). Returns the most relevant \
         recent matches — each with its thread id, sender, timestamp, and a \
         snippet. Use to recall what the user said, asked, or decided in earlier \
         chats before answering."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Keywords or phrase to match against past messages. Be specific — short common words are ignored."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max matches to return (default 8, max 25).",
                    "minimum": 1,
                    "maximum": 25
                },
                "exclude_thread_id": {
                    "type": "string",
                    "description": "Thread id to omit from results. Defaults to the active thread when omitted, so the message the user just sent does not crowd out the prior chats you are trying to recall. Pass a specific id to override, or an empty string to search every thread."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = read_required_str(&args, "query")?;
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|n| (n as usize).clamp(1, Self::MAX_LIMIT))
            .unwrap_or(Self::DEFAULT_LIMIT);
        // Resolve the effective thread to exclude:
        //   - arg omitted     → default to the ACTIVE thread, so the message the
        //                        user just sent (persisted before inference starts)
        //                        doesn't become the top "past conversation" hit and
        //                        crowd out the real prior chats we're recalling.
        //   - arg = "" (empty) → explicit opt-out: search every thread.
        //   - arg = "<id>"     → exclude that specific thread.
        let exclude_owned: Option<String> = match args
            .get("exclude_thread_id")
            .and_then(serde_json::Value::as_str)
        {
            None => crate::openhuman::inference::provider::thread_context::current_thread_id(),
            Some(s) if s.trim().is_empty() => None,
            Some(s) => Some(s.trim().to_string()),
        };
        let exclude = exclude_owned.as_deref();
        log::debug!(
            "[tool][threads] transcript_search invoked query_chars={} limit={} exclude={:?}",
            query.chars().count(),
            limit,
            exclude
        );

        let hits = ops::transcript_search(&query, limit, exclude)
            .await
            .map_err(|e| anyhow::anyhow!("transcript_search: {e}"))?;

        if hits.is_empty() {
            return Ok(ToolResult::success(format!(
                "No past messages matched `{query}`."
            )));
        }

        let mut out = format!(
            "{} past message(s) matched `{query}` (newest first):\n",
            hits.len()
        );
        for hit in &hits {
            let _ = std::fmt::Write::write_fmt(
                &mut out,
                format_args!(
                    "- [thread {} · {} · {}] {}\n",
                    hit.thread_id,
                    hit.role,
                    hit.created_at,
                    Self::snippet(&hit.content)
                ),
            );
        }
        Ok(ToolResult::success(out))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Append a message to a thread.
pub struct ThreadMessageAppendTool;

#[async_trait]
impl Tool for ThreadMessageAppendTool {
    fn name(&self) -> &str {
        "thread_message_append"
    }

    fn description(&self) -> &str {
        "Append a message record to a conversation thread. Requires `thread_id` \
         and a `message` object (id, content, type, sender, created_at)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "message": { "type": "object", "description": "ConversationMessageRecord." }
            },
            "required": ["thread_id", "message"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] message_append invoked");
        let req: AppendConversationMessageRequest = parse_req(args, "thread_message_append")?;
        let outcome = ops::message_append(req)
            .await
            .map_err(|e| anyhow::anyhow!("thread_message_append: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

/// Patch a message's metadata.
pub struct ThreadMessageUpdateTool;

#[async_trait]
impl Tool for ThreadMessageUpdateTool {
    fn name(&self) -> &str {
        "thread_message_update"
    }

    fn description(&self) -> &str {
        "Patch a message's `extra_metadata` in a thread. Requires `thread_id` \
         and `message_id`; `extra_metadata` is the new metadata object."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "message_id": { "type": "string" },
                "extra_metadata": { "type": "object" }
            },
            "required": ["thread_id", "message_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] message_update invoked");
        let req: UpdateConversationMessageRequest = parse_req(args, "thread_message_update")?;
        emit!(ops::message_update(req).await, "thread_message_update")
    }
}

/// AI-generate a thread title.
pub struct ThreadTitleGenerateTool;

#[async_trait]
impl Tool for ThreadTitleGenerateTool {
    fn name(&self) -> &str {
        "thread_title_generate"
    }

    fn description(&self) -> &str {
        "Generate a concise title for a conversation thread using the model, \
         based on its messages. Requires `thread_id`; optional \
         `assistant_message` to bias the title. Skips if a non-placeholder \
         title already exists."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "assistant_message": { "type": "string" }
            },
            "required": ["thread_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // Runs an inference call.
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] title_generate invoked");
        let req: GenerateConversationThreadTitleRequest = parse_req(args, "thread_title_generate")?;
        let outcome = ops::thread_generate_title(req)
            .await
            .map_err(|e| anyhow::anyhow!("thread_title_generate: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }
}

/// Read in-flight turn state for one thread.
pub struct ThreadTurnStateGetTool;

#[async_trait]
impl Tool for ThreadTurnStateGetTool {
    fn name(&self) -> &str {
        "thread_turn_state_get"
    }

    fn description(&self) -> &str {
        "Read the saved in-flight turn state for a thread by `thread_id` (the \
         snapshot of an interrupted/streaming agent turn), or null if none."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "thread_id": { "type": "string" } },
            "required": ["thread_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] turn_state_get invoked");
        let req: GetTurnStateRequest = parse_req(args, "thread_turn_state_get")?;
        emit!(ops::turn_state_get(req).await, "thread_turn_state_get")
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// List all saved turn-state snapshots.
pub struct ThreadTurnStateListTool;

#[async_trait]
impl Tool for ThreadTurnStateListTool {
    fn name(&self) -> &str {
        "thread_turn_state_list"
    }

    fn description(&self) -> &str {
        "List all saved in-flight turn-state snapshots across threads."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] turn_state_list invoked");
        emit!(
            ops::turn_state_list(EmptyRequest {}).await,
            "thread_turn_state_list"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Clear a thread's turn-state snapshot.
pub struct ThreadTurnStateClearTool;

#[async_trait]
impl Tool for ThreadTurnStateClearTool {
    fn name(&self) -> &str {
        "thread_turn_state_clear"
    }

    fn description(&self) -> &str {
        "Clear (delete) the saved in-flight turn-state snapshot for a thread by \
         `thread_id`. Returns whether a snapshot existed."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "thread_id": { "type": "string" } },
            "required": ["thread_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] turn_state_clear invoked");
        let req: ClearTurnStateRequest = parse_req(args, "thread_turn_state_clear")?;
        emit!(ops::turn_state_clear(req).await, "thread_turn_state_clear")
    }
}

/// Read a thread's kanban task board.
pub struct ThreadTaskBoardReadTool {
    config: Arc<Config>,
}

impl ThreadTaskBoardReadTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ThreadTaskBoardReadTool {
    fn name(&self) -> &str {
        "thread_task_board_read"
    }

    fn description(&self) -> &str {
        "Read a thread's kanban task board by `thread_id`, returning its cards \
         (empty board if none exists yet)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "thread_id": { "type": "string" } },
            "required": ["thread_id"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] task_board_read invoked");
        let thread_id = read_required_str(&args, "thread_id")?;
        let board = board_for_thread(&self.config.workspace_dir, &thread_id)
            .map_err(|e| anyhow::anyhow!("thread_task_board_read: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "task_board": board,
        }))?))
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Replace a thread's kanban task board.
pub struct ThreadTaskBoardWriteTool {
    config: Arc<Config>,
}

impl ThreadTaskBoardWriteTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ThreadTaskBoardWriteTool {
    fn name(&self) -> &str {
        "thread_task_board_write"
    }

    fn description(&self) -> &str {
        "Replace a thread's kanban task board with the supplied `cards` array \
         (TaskBoardCard objects). Requires `thread_id`. The board is normalized \
         and persisted; returns the saved board."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "cards": { "type": "array", "items": { "type": "object" } }
            },
            "required": ["thread_id", "cards"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] task_board_write invoked");
        let thread_id = read_required_str(&args, "thread_id")?;
        let cards_val = args
            .get("cards")
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("missing required array argument `cards`"))?;
        let cards: Vec<TaskBoardCard> = serde_json::from_value(cards_val)
            .map_err(|e| anyhow::anyhow!("thread_task_board_write: invalid cards: {e}"))?;
        let board = TaskBoard {
            thread_id,
            cards,
            updated_at: Utc::now().to_rfc3339(),
        };
        let saved = TaskBoardStore::new(self.config.workspace_dir.clone())
            .put(board)
            .map_err(|e| anyhow::anyhow!("thread_task_board_write: {e}"))?;
        Ok(ToolResult::success(serde_json::to_string(&json!({
            "task_board": saved,
        }))?))
    }
}

/// Delete one thread. **Irreversible** — default-OFF.
pub struct ThreadDeleteTool;

#[async_trait]
impl Tool for ThreadDeleteTool {
    fn name(&self) -> &str {
        "thread_delete"
    }

    fn description(&self) -> &str {
        "Permanently delete a conversation thread (and its messages + turn \
         state) by `thread_id`. Requires `deleted_at` (RFC3339 timestamp). \
         Irreversible. Only use when the user asks to delete a specific thread."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "thread_id": { "type": "string" },
                "deleted_at": { "type": "string", "description": "RFC3339 deletion timestamp." }
            },
            "required": ["thread_id", "deleted_at"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] delete invoked");
        let req: DeleteConversationThreadRequest = parse_req(args, "thread_delete")?;
        emit!(ops::thread_delete(req).await, "thread_delete")
    }
}

/// Purge every thread. **Irreversible** — default-OFF.
pub struct ThreadPurgeAllTool;

#[async_trait]
impl Tool for ThreadPurgeAllTool {
    fn name(&self) -> &str {
        "thread_purge_all"
    }

    fn description(&self) -> &str {
        "Delete EVERY conversation thread and message, and clear all turn-state \
         snapshots. Irreversible and total. Only use when the user explicitly \
         asks to wipe all conversation history."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][threads] purge_all invoked");
        emit!(
            ops::threads_purge(EmptyRequest {}).await,
            "thread_purge_all"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    fn cfg() -> Arc<Config> {
        Arc::new(Config::default())
    }

    #[test]
    fn names_and_levels() {
        assert_eq!(ThreadListTool.name(), "thread_list");
        assert_eq!(ThreadListTool.permission_level(), PermissionLevel::ReadOnly);
        assert_eq!(ThreadCreateTool.permission_level(), PermissionLevel::Write);
        assert_eq!(
            ThreadTitleGenerateTool.permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            ThreadDeleteTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(
            ThreadPurgeAllTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(
            ThreadTaskBoardReadTool::new(cfg()).permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            ThreadTaskBoardWriteTool::new(cfg()).permission_level(),
            PermissionLevel::Write
        );
        assert_eq!(ThreadListTool.scope(), ToolScope::All);
        assert_eq!(ThreadTranscriptSearchTool.name(), "transcript_search");
        assert_eq!(
            ThreadTranscriptSearchTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert!(ThreadTranscriptSearchTool.is_concurrency_safe(&json!({})));
    }

    #[test]
    fn transcript_search_requires_query() {
        let schema = ThreadTranscriptSearchTool.parameters_schema();
        let required = schema["required"].as_array().expect("required array");
        assert!(required.iter().any(|v| v == "query"));
    }

    #[test]
    fn transcript_search_snippet_collapses_and_truncates() {
        // Short content is returned with whitespace collapsed, untruncated.
        let short = ThreadTranscriptSearchTool::snippet("hello   there\n\nworld");
        assert_eq!(short, "hello there world");
        // Long content is cut to the cap with an ellipsis.
        let long_src = "x ".repeat(400);
        let long = ThreadTranscriptSearchTool::snippet(&long_src);
        assert!(long.ends_with('…'));
        assert!(long.chars().count() <= ThreadTranscriptSearchTool::SNIPPET_CHARS + 1);
    }

    #[test]
    fn find_thread_obj_matches_nested_id() {
        let blob = json!({
            "data": { "threads": [ { "id": "t1", "title": "A" }, { "id": "t2" } ] }
        });
        let found = find_thread_obj(&blob, "t2").expect("found t2");
        assert_eq!(found["id"], "t2");
        assert!(find_thread_obj(&blob, "nope").is_none());
    }

    #[tokio::test]
    async fn update_title_requires_args() {
        let err = ThreadUpdateTitleTool
            .execute(json!({ "thread_id": "t1" }))
            .await
            .expect_err("missing title");
        assert!(err.to_string().contains("thread_update_title"));
    }

    #[tokio::test]
    async fn delete_requires_deleted_at() {
        let err = ThreadDeleteTool
            .execute(json!({ "thread_id": "t1" }))
            .await
            .expect_err("missing deleted_at");
        assert!(err.to_string().contains("thread_delete"));
    }

    #[tokio::test]
    async fn task_board_read_requires_thread_id() {
        let err = ThreadTaskBoardReadTool::new(cfg())
            .execute(json!({}))
            .await
            .expect_err("missing thread_id");
        assert!(err.to_string().contains("thread_id"));
    }
}
