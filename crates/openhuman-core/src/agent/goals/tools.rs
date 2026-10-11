//! Host wiring for tinyagents' model-facing goal controls.
//!
//! `goal_get` / `goal_set` / `goal_complete` are owned by
//! [`tinyagents_graph::goals`] (`GoalTool`); ownership stays asymmetric
//! (pause / resume / budget-limit are system-driven and have no model tool).
//! This module only supplies what is OpenHuman's: the workspace-backed goal
//! store and the `ThreadGoalUpdated` domain event the UI chip listens to.

use std::path::Path;
use std::sync::Arc;

use tinyagents_graph::goals::{GoalTool, GoalToolKind, GoalUpdateHook};
use tinytools::Tool;

use {super::goal_to_value, super::store, tinyagents_graph::goals::ThreadGoal};

/// The model-facing goal tools for `workspace_dir`, each publishing
/// `DomainEvent::ThreadGoalUpdated` after a write.
pub fn goal_tools(workspace_dir: &Path) -> Vec<Box<dyn Tool>> {
    let goal_store = store::goals_store(workspace_dir);
    let hook: GoalUpdateHook = Arc::new(publish_goal_updated);
    GoalToolKind::MODEL_FACING
        .into_iter()
        .map(|kind| {
            Box::new(GoalTool::new(kind, goal_store.clone()).with_update_hook(hook.clone()))
                as Box<dyn Tool>
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
