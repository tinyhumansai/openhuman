//! Agent-facing tools for the thread-level goal.
//!
//! These let the orchestrator (and any agent that allowlists them) read and
//! drive the current thread's goal. Ownership is **asymmetric** (Codex parity):
//! the model may create/replace the goal (`goal_set`), read it (`goal_get`), and
//! mark it complete (`goal_complete`). Pause / resume / budget-limit are
//! system-driven and have no model tool.
//!
//! The target thread is resolved from the ambient
//! [`current_thread_id`](crate::openhuman::inference::provider::thread_context::current_thread_id)
//! task-local set by the chat channel — tools never take a `thread_id` arg, so
//! the model can't address another thread's goal. Each tool is sandboxed to a
//! single `workspace_dir` captured at construction.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::json;

use super::store;
use super::types::ThreadGoal;
use crate::openhuman::inference::provider::thread_context::current_thread_id;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

/// Render a goal as a compact, model-readable block.
fn render_goal(goal: &ThreadGoal) -> String {
    let budget = match (goal.token_budget, goal.budget_remaining()) {
        (Some(b), Some(rem)) => format!("{} used / {b} budget ({rem} left)", goal.tokens_used),
        _ => format!("{} used / no budget", goal.tokens_used),
    };
    format!(
        "[thread_goal]\nstatus: {}\nobjective: {}\ntokens: {budget}\n[/thread_goal]",
        goal.status.as_str(),
        goal.objective
    )
}

/// Resolve the ambient thread id or return a uniform tool error.
fn require_thread_id() -> Result<String, ToolResult> {
    current_thread_id().ok_or_else(|| {
        ToolResult::error(
            "thread goal tools require an active chat thread (no ambient thread_id in this context)",
        )
    })
}

/// `goal_get` — read the current thread goal.
pub struct GoalGetTool {
    workspace_dir: PathBuf,
}

impl GoalGetTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalGetTool {
    fn name(&self) -> &str {
        "goal_get"
    }

    fn description(&self) -> &str {
        "Read this thread's goal — the durable objective you're pursuing across \
         turns — with its status (active/paused/budget_limited/complete) and token \
         usage. Returns 'no goal set' when the thread has none."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let thread_id = match require_thread_id() {
            Ok(id) => id,
            Err(e) => return Ok(e),
        };
        log::debug!("[thread_goals] tool=goal_get thread_id={thread_id}");
        match store::get(&self.workspace_dir, &thread_id).await {
            Ok(Some(goal)) => Ok(ToolResult::success(render_goal(&goal))),
            Ok(None) => Ok(ToolResult::success("no goal set for this thread")),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// `goal_set` — create or replace this thread's goal.
pub struct GoalSetTool {
    workspace_dir: PathBuf,
}

impl GoalSetTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalSetTool {
    fn name(&self) -> &str {
        "goal_set"
    }

    fn description(&self) -> &str {
        "Set (or replace) this thread's goal — the durable objective you should \
         keep pursuing across turns until it's complete. Use at the start of a \
         non-trivial request, or to refine the objective as it sharpens. Changing \
         the objective resets usage counters. Optionally set a token_budget; when \
         reached, the goal pauses with a progress summary."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "required": ["objective"],
            "properties": {
                "objective": {
                    "type": "string",
                    "description": "The durable objective — what 'done' looks like for this thread."
                },
                "token_budget": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Optional token ceiling for the goal. Omit for no limit."
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let thread_id = match require_thread_id() {
            Ok(id) => id,
            Err(e) => return Ok(e),
        };
        let Some(objective) = args.get("objective").and_then(|v| v.as_str()) else {
            return Ok(ToolResult::error("Missing 'objective' parameter"));
        };
        let token_budget = args.get("token_budget").and_then(|v| v.as_u64());
        log::debug!("[thread_goals] tool=goal_set thread_id={thread_id} budget={token_budget:?}");
        match store::set(&self.workspace_dir, &thread_id, objective, token_budget).await {
            Ok(goal) => {
                // Emit the live-update event so the UI chip refreshes immediately.
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ThreadGoalUpdated {
                        thread_id: goal.thread_id.clone(),
                        goal_id: goal.goal_id.clone(),
                        status: goal.status.as_str().to_string(),
                    },
                );
                Ok(ToolResult::success(format!(
                    "Goal set.\n{}",
                    render_goal(&goal)
                )))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// `goal_complete` — mark this thread's goal complete (evidence-backed success).
pub struct GoalCompleteTool {
    workspace_dir: PathBuf,
}

impl GoalCompleteTool {
    pub fn new(workspace_dir: PathBuf) -> Self {
        Self { workspace_dir }
    }
}

#[async_trait]
impl Tool for GoalCompleteTool {
    fn name(&self) -> &str {
        "goal_complete"
    }

    fn description(&self) -> &str {
        "Mark this thread's goal complete. Only call this when concrete evidence \
         confirms the objective is satisfied — completing stops autonomous \
         continuation."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let thread_id = match require_thread_id() {
            Ok(id) => id,
            Err(e) => return Ok(e),
        };
        log::debug!("[thread_goals] tool=goal_complete thread_id={thread_id}");
        match store::complete(&self.workspace_dir, &thread_id).await {
            Ok(goal) => {
                crate::core::event_bus::publish_global(
                    crate::core::event_bus::DomainEvent::ThreadGoalUpdated {
                        thread_id: goal.thread_id.clone(),
                        goal_id: goal.goal_id.clone(),
                        status: goal.status.as_str().to_string(),
                    },
                );
                Ok(ToolResult::success(format!(
                    "Goal marked complete.\n{}",
                    render_goal(&goal)
                )))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::thread_context::with_thread_id;

    #[tokio::test]
    async fn set_get_complete_via_tools_in_thread_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        with_thread_id("thread-tools", async {
            let set = GoalSetTool::new(dir.clone());
            let res = set
                .execute(json!({ "objective": "land the PR", "token_budget": 5000 }))
                .await
                .unwrap();
            assert!(!res.is_error, "{}", res.text());
            assert!(res.text().contains("land the PR"));

            let get = GoalGetTool::new(dir.clone());
            let res = get.execute(json!({})).await.unwrap();
            assert!(res.text().contains("status: active"));

            let done = GoalCompleteTool::new(dir.clone());
            let res = done.execute(json!({})).await.unwrap();
            assert!(res.text().contains("status: complete"));
        })
        .await;
    }

    #[tokio::test]
    async fn tools_error_without_thread_scope() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let set = GoalSetTool::new(dir.clone());
        let res = set.execute(json!({ "objective": "x" })).await.unwrap();
        assert!(res.is_error);
        assert!(res.text().contains("active chat thread"));
    }

    #[tokio::test]
    async fn get_reports_absent_goal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        with_thread_id("empty-thread", async {
            let get = GoalGetTool::new(dir.clone());
            let res = get.execute(json!({})).await.unwrap();
            assert!(res.text().contains("no goal set"));
        })
        .await;
    }
}
