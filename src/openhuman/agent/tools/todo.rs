//! `todo` — unified CRUD tool for the agent's task board.
//!
//! Dispatches on the `op` field so a single tool exposes
//! `add` / `edit` / `update_status` / `remove` / `replace` / `clear` /
//! `list`. The board is persisted to the active thread (when there is
//! one) via [`crate::openhuman::todos::ops`]; without a thread context the
//! tool falls back to a process-global scratch list. Returns a markdown
//! rendering so transcripts read cleanly.

use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard, TaskCardStatus};
use crate::openhuman::inference::provider::thread_context;
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;

pub struct TodoTool;

impl TodoTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for TodoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn name(&self) -> &str {
        "todo"
    }

    fn description(&self) -> &str {
        "Maintain a visible plan for THIS conversation thread (the cards render \
         above the user's composer and survive across turns). \
         Use it for any request with 3+ steps or several distinct tasks: at the \
         start, `add` one card per step up front; keep exactly ONE card \
         `in_progress` at a time; mark a card `done` the moment it is finished \
         (do not batch completions); if a step fails or is abandoned, set it \
         `blocked` with a `blocker`, or revise it. `list` to re-read the current \
         plan when resuming. Skip this tool for trivial single-step requests. \
         The board is bound automatically to the current thread — do not pass a \
         thread id. \
         Dispatch via the `op` field: \
         `add` (content, status?, objective?, plan?, assignedAgent?, allowedTools?, \
         approvalMode?, acceptanceCriteria?, evidence?, notes?, blocker?), \
         `edit` (id, content?, status?, objective?, plan?, assignedAgent?, allowedTools?, \
         approvalMode?, acceptanceCriteria?, evidence?, notes?, blocker?), \
         `update_status` (id, status), \
         `remove` (id), \
         `replace` (cards: full list — wholesale replace), \
         `clear`, or `list`. \
         `status` is one of `todo` / `in_progress` / `blocked` / `done`. \
         Returns the updated list as cards plus a markdown rendering."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "op": {
                    "type": "string",
                    "enum": ["add", "edit", "update_status", "remove", "replace", "clear", "list"]
                },
                "id": { "type": "string", "description": "Card id (required for edit/update_status/remove)." },
                "content": { "type": "string", "description": "Card title (required for add; optional for edit)." },
                "status": {
                    "type": "string",
                    "enum": ["todo", "pending", "in_progress", "blocked", "done", "completed"]
                },
                "notes": { "type": "string" },
                "blocker": { "type": "string" },
                "objective": { "type": "string", "description": "Desired outcome for this task." },
                "plan": {
                    "type": "array",
                    "description": "Ordered lightweight execution steps.",
                    "items": { "type": "string" }
                },
                "assignedAgent": { "type": "string", "description": "Agent id expected to pick up this task." },
                "allowedTools": {
                    "type": "array",
                    "description": "Task-local tool names or toolkit slugs the assigned agent may use.",
                    "items": { "type": "string" }
                },
                "approvalMode": {
                    "type": ["string", "null"],
                    "enum": ["required", "not_required", null]
                },
                "acceptanceCriteria": {
                    "type": "array",
                    "description": "Checklist that must be true before the task is done.",
                    "items": { "type": "string" }
                },
                "evidence": {
                    "type": "array",
                    "description": "Verification output, links, files, or notes produced while executing the task.",
                    "items": { "type": "string" }
                },
                "cards": {
                    "type": "array",
                    "description": "Full card list for op=replace.",
                    "items": { "type": "object" }
                }
            },
            "required": ["op"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let op = args
            .get("op")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required field `op`"))?
            .trim()
            .to_string();

        let location = current_location();
        tracing::debug!(op = %op, thread_id = ?location.thread_id(), "[tool][todo] dispatch");

        let result = match op.as_str() {
            "add" => {
                let content = required_string(&args, "content")?;
                let mut patch = patch_from_args(&args)?;
                if patch.approval_mode.is_none() {
                    patch.approval_mode = Some(default_task_approval_mode().await);
                }
                ops::add(&location, &content, patch)
            }
            "edit" => {
                let id = required_string(&args, "id")?;
                let mut patch = patch_from_args(&args)?;
                patch.content = optional_string(&args, "content");
                ops::edit(&location, &id, patch)
            }
            "update_status" => {
                let id = required_string(&args, "id")?;
                let status = required_string(&args, "status")?;
                let status = ops::parse_status(&status).map_err(anyhow::Error::msg)?;
                ops::update_status(&location, &id, status)
            }
            "remove" => {
                let id = required_string(&args, "id")?;
                ops::remove(&location, &id)
            }
            "replace" => {
                let cards = args
                    .get("cards")
                    .ok_or_else(|| anyhow::anyhow!("missing `cards` for op=replace"))?;
                let cards: Vec<TaskBoardCard> = serde_json::from_value(cards.clone())
                    .map_err(|e| anyhow::anyhow!("invalid `cards`: {e}"))?;
                ops::replace(&location, cards)
            }
            "clear" => ops::clear(&location),
            "list" => ops::list(&location),
            other => {
                return Ok(ToolResult::error(format!(
                "unknown op '{other}' (expected add|edit|update_status|remove|replace|clear|list)"
            )))
            }
        };

        match result {
            Ok(snap) => {
                let payload = json!({
                    "threadId": snap.thread_id,
                    "cards": snap.cards,
                    "markdown": snap.markdown,
                });
                Ok(ToolResult::success(payload.to_string()))
            }
            Err(err) => Ok(ToolResult::error(err)),
        }
    }
}

async fn default_task_approval_mode() -> Option<TaskApprovalMode> {
    match crate::openhuman::config::ops::load_config_with_timeout().await {
        Ok(config) => Some(if config.autonomy.require_task_plan_approval {
            TaskApprovalMode::Required
        } else {
            TaskApprovalMode::NotRequired
        }),
        Err(err) => {
            tracing::debug!(
                error = %err,
                "[tool][todo] failed to load config for task approval default"
            );
            None
        }
    }
}

fn current_location() -> BoardLocation {
    let Some(parent) = crate::openhuman::agent::harness::fork_context::current_parent() else {
        return BoardLocation::Scratch;
    };
    let Some(thread_id) = thread_context::current_thread_id() else {
        return BoardLocation::Scratch;
    };
    BoardLocation::Thread {
        workspace_dir: parent.workspace_dir.clone(),
        thread_id,
    }
}

fn required_string(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing required field `{key}`"))?;
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("missing required field `{key}`"));
    }
    Ok(trimmed.to_string())
}

fn optional_string(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

fn patch_from_args(args: &serde_json::Value) -> anyhow::Result<CardPatch> {
    let status: Option<TaskCardStatus> = match args.get("status").and_then(|v| v.as_str()) {
        Some(s) => Some(ops::parse_status(s).map_err(anyhow::Error::msg)?),
        None => None,
    };
    let approval_mode = match args.get("approvalMode") {
        Some(value) if value.is_null() => Some(None),
        Some(value) => match value.as_str() {
            Some("required") => Some(Some(TaskApprovalMode::Required)),
            Some("not_required") => Some(Some(TaskApprovalMode::NotRequired)),
            Some(other) => {
                return Err(anyhow::anyhow!(
                    "invalid approvalMode '{other}' (expected required|not_required|null)"
                ))
            }
            None => {
                return Err(anyhow::anyhow!(
                    "invalid approvalMode type (expected required|not_required|null)"
                ))
            }
        },
        None => None,
    };
    Ok(CardPatch {
        content: None,
        status,
        objective: optional_string(args, "objective"),
        plan: optional_string_array(args, "plan")?,
        assigned_agent: optional_string(args, "assignedAgent"),
        allowed_tools: optional_string_array(args, "allowedTools")?,
        approval_mode,
        acceptance_criteria: optional_string_array(args, "acceptanceCriteria")?,
        evidence: optional_string_array(args, "evidence")?,
        notes: optional_string(args, "notes"),
        blocker: optional_string(args, "blocker"),
        source_metadata: None,
    })
}

fn optional_string_array(
    args: &serde_json::Value,
    key: &str,
) -> anyhow::Result<Option<Vec<String>>> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let values = value
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("`{key}` must be an array of strings"))?;
    values
        .iter()
        .map(|item| {
            item.as_str()
                .map(|s| s.to_string())
                .ok_or_else(|| anyhow::anyhow!("`{key}` must be an array of strings"))
        })
        .collect::<anyhow::Result<Vec<_>>>()
        .map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::todos::global_scratch_store;
    use serde_json::Value;

    /// Serialize tests that share the process-global scratch store with
    /// `todos::ops` tests. Same lock — otherwise the two test modules race
    /// under `cargo test`'s thread pool.
    fn scratch_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::todos::ops::scratch_test_lock()
    }

    fn reset_scratch() {
        global_scratch_store().replace(Vec::new());
    }

    #[tokio::test]
    async fn add_then_list_round_trips_via_scratch() {
        let _guard = scratch_lock();
        reset_scratch();
        let tool = TodoTool::new();
        let added = tool
            .execute(json!({ "op": "add", "content": "Write tests" }))
            .await
            .unwrap();
        assert!(!added.is_error, "{}", added.output());
        let payload: Value = serde_json::from_str(&added.output()).unwrap();
        let cards = payload["cards"].as_array().unwrap();
        assert_eq!(cards.len(), 1);
        let id = cards[0]["id"].as_str().unwrap().to_string();
        assert!(payload["markdown"]
            .as_str()
            .unwrap()
            .contains("[ ] Write tests"));

        let listed = tool.execute(json!({ "op": "list" })).await.unwrap();
        let listed_payload: Value = serde_json::from_str(&listed.output()).unwrap();
        assert_eq!(listed_payload["cards"].as_array().unwrap().len(), 1);

        let done = tool
            .execute(json!({ "op": "update_status", "id": id, "status": "done" }))
            .await
            .unwrap();
        let done_payload: Value = serde_json::from_str(&done.output()).unwrap();
        assert!(done_payload["markdown"]
            .as_str()
            .unwrap()
            .contains("[x] Write tests"));
        reset_scratch();
    }

    #[tokio::test]
    async fn unknown_op_returns_error() {
        let tool = TodoTool::new();
        let result = tool.execute(json!({ "op": "frobnicate" })).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("unknown op"));
    }

    #[tokio::test]
    async fn add_requires_content() {
        let tool = TodoTool::new();
        let err = tool.execute(json!({ "op": "add" })).await.unwrap_err();
        assert!(err.to_string().contains("content"));
    }

    #[test]
    fn description_carries_planning_guidance() {
        // The `todo` tool steers the live orchestrator purely through its static
        // (prompt-cache-stable) schema description — there is no per-thread prompt
        // injection. Lock in the behavioural contract so the guidance can't be
        // silently dropped: when-to-use, single-in_progress discipline, and the
        // "bound to the current thread, don't pass a thread id" rule.
        let tool = TodoTool::new();
        let desc = tool.description();
        assert!(desc.contains("3+ steps"), "missing when-to-use guidance");
        assert!(
            desc.contains("ONE card `in_progress`"),
            "missing single-in_progress discipline"
        );
        assert!(
            desc.contains("do not pass a thread id"),
            "missing explicit 'do not pass a thread id' note"
        );
    }

    #[tokio::test]
    async fn edit_rejects_unknown_id() {
        let _guard = scratch_lock();
        reset_scratch();
        let tool = TodoTool::new();
        let result = tool
            .execute(json!({ "op": "edit", "id": "task-missing", "content": "x" }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("not found"));
        reset_scratch();
    }

    #[tokio::test]
    async fn replace_accepts_full_card_list() {
        let _guard = scratch_lock();
        reset_scratch();
        let tool = TodoTool::new();
        let result = tool
            .execute(json!({
                "op": "replace",
                "cards": [
                    {
                        "id": "",
                        "title": "Alpha",
                        "status": "todo",
                        "order": 0,
                        "updated_at": ""
                    },
                    {
                        "id": "",
                        "title": "Beta",
                        "status": "in_progress",
                        "order": 1,
                        "updated_at": ""
                    }
                ]
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "{}", result.output());
        let payload: Value = serde_json::from_str(&result.output()).unwrap();
        assert_eq!(payload["cards"].as_array().unwrap().len(), 2);
        reset_scratch();
    }
}
