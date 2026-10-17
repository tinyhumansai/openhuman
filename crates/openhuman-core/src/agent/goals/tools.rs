//! Host wiring for tinyagents' model-facing goal controls.
//!
//! `goal_get` / `goal_set` / `goal_complete` are owned by
//! [`tinyagents_graph::goals`] (`GoalTool`); ownership stays asymmetric
//! (pause / resume / budget-limit are system-driven and have no model tool).
//! This module only supplies what is OpenHuman's: the workspace-backed goal
//! store and the `ThreadGoalUpdated` domain event the UI chip listens to.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyagents_graph::goals::{GoalTool, GoalToolKind, GoalUpdateHook};
use tinytools::{
    PermissionLevel, Tool, ToolCallOptions, ToolInjectedArgument, ToolPolicy, ToolResult,
    ToolRunContext,
};

use super::{goal_to_value, store, ThreadGoal};

/// OpenHuman host wrapper around tinyagents' [`GoalTool`], enforcing:
/// 1. `goal_complete` cannot bypass review or finish when an autonomous task is active on the thread.
/// 2. `goal_set` cannot be called from background/automated turns (only user-initiated turns).
pub struct HostGoalTool {
    inner: GoalTool,
    workspace_dir: PathBuf,
}

impl HostGoalTool {
    pub fn new(inner: GoalTool, workspace_dir: PathBuf) -> Self {
        Self {
            inner,
            workspace_dir,
        }
    }
}

#[async_trait]
impl Tool for HostGoalTool {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn parameters_schema(&self) -> Value {
        self.inner.parameters_schema()
    }

    fn supports_markdown(&self) -> bool {
        self.inner.supports_markdown()
    }

    fn permission_level(&self) -> PermissionLevel {
        self.inner.permission_level()
    }

    fn policy(&self) -> ToolPolicy {
        self.inner.policy()
    }

    fn injected_arguments(&self) -> Vec<ToolInjectedArgument> {
        self.inner.injected_arguments()
    }

    async fn prepare_authorization(&self, args: &Value) -> anyhow::Result<Option<Value>> {
        self.inner.prepare_authorization(args).await
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None).await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        if let Some(thread_id) = context.and_then(ToolRunContext::thread_id) {
            if self.inner.kind() == GoalToolKind::Complete {
                if let Ok(Some(work)) = crate::agent::autonomous::store::get(&self.workspace_dir, thread_id).await {
                    if work.runnable() || work.phase != crate::agent::autonomous::types::Phase::Complete {
                        return Ok(ToolResult::error(
                            "An autonomous work task exists on this thread; goal_complete cannot bypass review. Use autonomous_work review/finish.",
                        ));
                    }
                }
            }

            if self.inner.kind() == GoalToolKind::Set {
                if let Some(origin) = crate::agent::turn_origin::current() {
                    if !origin.is_user_authored() {
                        return Ok(ToolResult::error(
                            "Background turns cannot set or resume goal budgets; only user-initiated turns can update goals.",
                        ));
                    }
                }
            }
        }

        self.inner.execute_with_context(args, options, context).await
    }
}

/// The model-facing goal tools for `workspace_dir`, each publishing
/// `DomainEvent::ThreadGoalUpdated` after a write.
pub fn goal_tools(workspace_dir: &Path) -> Vec<Box<dyn Tool>> {
    let goal_store = store::goals_store(workspace_dir);
    let hook: GoalUpdateHook = Arc::new(publish_goal_updated);
    GoalToolKind::MODEL_FACING
        .into_iter()
        .map(|kind| {
            let inner = GoalTool::new(kind, goal_store.clone()).with_update_hook(hook.clone());
            Box::new(HostGoalTool::new(inner, workspace_dir.to_path_buf())) as Box<dyn Tool>
        })
        .collect()
}

/// Emit the live-update event so the UI chip refreshes immediately.
fn publish_goal_updated(goal: &ThreadGoal) {
    tracing::debug!(
        thread_id = %goal.thread_id,
        goal_id = %goal.goal_id,
        status = goal.status.as_str(),
        "[thread_goals] tool wrote goal, publishing ThreadGoalUpdated"
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ThreadGoalUpdated {
        thread_id: goal.thread_id.clone(),
        goal_id: goal.goal_id.clone(),
        status: goal.status.as_str().to_string(),
        goal: Some(goal_to_value(goal)),
    });
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
