//! `update_task` — move or update a specific task card on a thread's board.
//!
//! `todo` only reaches the *current* thread's board. `update_task` addresses a
//! card **by id** on a *target* board — defaulting to the proactive
//! `task-sources` board — so the orchestrator (or an autonomous task run on its
//! own thread) can advance the task it's working: move it to
//! `in_progress`/`blocked`/`done`, or update its objective/notes/evidence/blocker.
//!
//! It is a thin wrapper over [`crate::openhuman::todos::ops::edit`], which
//! applies the status move + field updates atomically, enforces the
//! single-`in_progress` invariant, and emits the board-progress event that the
//! Tasks board UI listens on.

use crate::openhuman::task_sources::TASK_SOURCES_THREAD_ID;
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;

pub struct UpdateTaskTool;

impl UpdateTaskTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for UpdateTaskTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for UpdateTaskTool {
    fn name(&self) -> &str {
        "update_task"
    }

    fn description(&self) -> &str {
        "Move or update a specific task card on a task board, addressed by `id`. \
         Use this to advance the task you're working on: set `status` \
         (`todo`/`in_progress`/`blocked`/`done`) to move it between columns, and/or \
         update `objective`, `notes`, `evidence`, `blocker`, `plan`, \
         `acceptanceCriteria`. When you finish, set `status: done` with `evidence`; \
         if you cannot proceed, set `status: blocked` with a `blocker` reason. \
         Targets the proactive `task-sources` board by default — pass `threadId` to \
         target another thread's board. Returns the updated card list + a markdown \
         rendering. At most one card may be `in_progress` at a time."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Id of the card to move/update (required)." },
                "status": {
                    "type": "string",
                    "enum": ["todo", "in_progress", "blocked", "done"],
                    "description": "New status — moves the card to that column."
                },
                "objective": { "type": "string", "description": "Updated desired outcome for the task." },
                "notes": { "type": "string", "description": "Progress notes / running summary." },
                "blocker": { "type": "string", "description": "Why the task is blocked (set with status=blocked)." },
                "evidence": {
                    "type": "array",
                    "description": "Links, output, or files proving the work (set with status=done).",
                    "items": { "type": "string" }
                },
                "plan": {
                    "type": "array",
                    "description": "Updated ordered execution steps.",
                    "items": { "type": "string" }
                },
                "acceptanceCriteria": {
                    "type": "array",
                    "description": "Updated checklist that must hold before the task is done.",
                    "items": { "type": "string" }
                },
                "threadId": {
                    "type": "string",
                    "description": "Board to target; defaults to the `task-sources` board."
                }
            },
            "required": ["id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::None
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let Some(id) = optional_string(&args, "id") else {
            return Ok(ToolResult::error("missing required field `id`".to_string()));
        };

        let patch = match build_patch(&args) {
            Ok(patch) => patch,
            Err(err) => return Ok(ToolResult::error(err)),
        };
        if patch_is_empty(&patch) {
            return Ok(ToolResult::error(
                "nothing to update — provide `status` and/or a field \
                 (objective/notes/evidence/blocker/plan/acceptanceCriteria)"
                    .to_string(),
            ));
        }

        let location = match resolve_location(&args).await {
            Ok(location) => location,
            Err(err) => return Ok(ToolResult::error(err)),
        };

        Ok(apply(&location, &id, patch))
    }
}

/// Apply the move/update to the card and render the result. Split out from
/// `execute` so the edit + response shaping is testable without a fork/thread
/// context (which `resolve_location` needs).
fn apply(location: &BoardLocation, id: &str, patch: CardPatch) -> ToolResult {
    tracing::info!(
        card_id = %id,
        thread_id = ?location.thread_id(),
        status = ?patch.status,
        "[tool][update_task] move/update task card"
    );
    match ops::edit(location, id, patch) {
        Ok(snap) => {
            let payload = json!({
                "threadId": snap.thread_id,
                "cards": snap.cards,
                "markdown": snap.markdown,
            });
            ToolResult::success(payload.to_string())
        }
        Err(err) => ToolResult::error(err),
    }
}

/// Resolve the board to act on: the explicit `threadId` arg, else the proactive
/// `task-sources` board. The workspace root comes from the running agent's fork
/// context when present, otherwise from the loaded config.
async fn resolve_location(args: &serde_json::Value) -> Result<BoardLocation, String> {
    let thread_id =
        optional_string(args, "threadId").unwrap_or_else(|| TASK_SOURCES_THREAD_ID.to_string());
    Ok(BoardLocation::Thread {
        workspace_dir: workspace_dir().await?,
        thread_id,
    })
}

async fn workspace_dir() -> Result<PathBuf, String> {
    if let Some(parent) = crate::openhuman::agent::harness::fork_context::current_parent() {
        return Ok(parent.workspace_dir.clone());
    }
    crate::openhuman::config::ops::load_config_with_timeout()
        .await
        .map(|config| config.workspace_dir)
        .map_err(|e| format!("update_task: failed to load config for workspace dir: {e}"))
}

fn build_patch(args: &serde_json::Value) -> Result<CardPatch, String> {
    let status = match args.get("status").and_then(|v| v.as_str()) {
        Some(s) => Some(ops::parse_status(s)?),
        None => None,
    };
    Ok(CardPatch {
        status,
        objective: optional_string(args, "objective"),
        plan: optional_string_array(args, "plan")?,
        acceptance_criteria: optional_string_array(args, "acceptanceCriteria")?,
        evidence: optional_string_array(args, "evidence")?,
        notes: optional_string(args, "notes"),
        blocker: optional_string(args, "blocker"),
        ..Default::default()
    })
}

fn patch_is_empty(patch: &CardPatch) -> bool {
    patch.status.is_none()
        && patch.objective.is_none()
        && patch.plan.is_none()
        && patch.acceptance_criteria.is_none()
        && patch.evidence.is_none()
        && patch.notes.is_none()
        && patch.blocker.is_none()
}

fn optional_string(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn optional_string_array(
    args: &serde_json::Value,
    key: &str,
) -> Result<Option<Vec<String>>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                let s = item
                    .as_str()
                    .ok_or_else(|| format!("`{key}` must be an array of strings"))?
                    .trim();
                if !s.is_empty() {
                    out.push(s.to_string());
                }
            }
            Ok(Some(out))
        }
        Some(_) => Err(format!("`{key}` must be an array of strings")),
    }
}

#[cfg(test)]
#[path = "update_task_tests.rs"]
mod tests;
