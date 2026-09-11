//! `todo` — unified CRUD tool for the agent's task board.
//!
//! Dispatches on the `op` field so a single tool exposes
//! `add` / `edit` / `update_status` / `remove` / `replace` / `clear` /
//! `list`. The board is persisted to the active thread (when there is
//! one) via [`crate::openhuman::threads::todos::ops`]; without a thread context the
//! tool falls back to a process-global scratch list. Returns a markdown
//! rendering so transcripts read cleanly.

use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard, TaskCardStatus};
use crate::openhuman::agent::tinyagents::thread_context;
use crate::openhuman::threads::todos::ops::{self, BoardLocation, CardPatch};
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
        "Maintain the visible plan for this thread; cards persist across turns. Use for requests with 3+ steps. Keep one `in_progress`; mark finished cards `done` immediately and blocked cards with a `blocker`. The board binds automatically; do not pass a thread id. Orchestrator calls use the shared board."
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
                ops::add(&location, &content, patch).await
            }
            "edit" => {
                let id = required_string(&args, "id")?;
                let mut patch = patch_from_args(&args)?;
                patch.content = optional_string(&args, "content");
                ops::edit(&location, &id, patch).await
            }
            "update_status" => {
                let id = required_string(&args, "id")?;
                let status = required_string(&args, "status")?;
                let status = ops::parse_status(&status).map_err(anyhow::Error::msg)?;
                ops::update_status(&location, &id, status).await
            }
            "remove" => {
                let id = required_string(&args, "id")?;
                ops::remove(&location, &id).await
            }
            "replace" => {
                let cards = args
                    .get("cards")
                    .ok_or_else(|| anyhow::anyhow!("missing `cards` for op=replace"))?;
                let cards: Vec<TaskBoardCard> = serde_json::from_value(cards.clone())
                    .map_err(|e| anyhow::anyhow!("invalid `cards`: {e}"))?;
                ops::replace(&location, cards).await
            }
            "clear" => ops::clear(&location).await,
            "list" => ops::list(&location).await,
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
    // Interactive plan review is handled by the `request_plan_review` gate
    // (it parks the live turn), NOT by stamping conversation-thread cards: the
    // background dispatcher never sweeps conversation boards, so a card status
    // can't gate a chat turn. This default therefore just carries the
    // config-driven behaviour for the dispatched boards (`user-tasks` /
    // `task-sources`).
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
    // The orchestrator owns ONE global task board rather than a per-thread one:
    // its `todo` tool always targets the app-wide `orchestrator-tasks` board so a
    // single Kanban spans every delegation (matches the UI's OrchestratorTaskBoard).
    if parent.agent_definition_id == "orchestrator" {
        return BoardLocation::Thread {
            workspace_dir: parent.workspace_dir.clone(),
            thread_id: ops::ORCHESTRATOR_TASKS_THREAD_ID.to_string(),
        };
    }
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
#[path = "todo_tests.rs"]
mod tests;
