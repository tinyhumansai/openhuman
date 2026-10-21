//! Host wiring for tinyagents' model-facing goal controls.
//!
//! `goal_get` / `goal_set` / `goal_complete` are owned by
//! [`tinyagents_harness_tools::goals`] (`GoalTool`); ownership stays asymmetric
//! (pause / resume / budget-limit are system-driven and have no model tool).
//! This module only supplies what is OpenHuman's: the workspace-backed goal
//! store and the `ThreadGoalUpdated` domain event the UI chip listens to.

use std::path::Path;
use std::sync::Arc;

use tinyagents_harness_tools::goals::{GoalObserver, GoalStore, GoalTool, GoalToolKind};
use tinytools::Tool;

use super::goal_to_value;
use tinyagents_harness_tools::goals::ThreadGoal;

/// The model-facing goal tools for `workspace_dir`, each publishing
/// `DomainEvent::ThreadGoalUpdated` after a write.
pub fn goal_tools(workspace_dir: &Path) -> Vec<Box<dyn Tool>> {
    let backing_store = crate::storage::sessions::stores_or_ephemeral(workspace_dir).kv;
    let goal_store = Arc::new(GoalStore::with_observer(
        backing_store,
        Arc::new(OpenHumanGoalObserver),
    ));
    GoalToolKind::MODEL_FACING
        .into_iter()
        .map(|kind| Box::new(GoalTool::new(kind, goal_store.clone())) as Box<dyn Tool>)
        .collect()
}

/// Emit live-update events so the UI chip refreshes after goal tool writes.
struct OpenHumanGoalObserver;

impl GoalObserver for OpenHumanGoalObserver {
    fn on_updated(&self, goal: &ThreadGoal) {
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

    fn on_cleared(&self, thread_id: &str) {
        crate::core::bus::BUS.publish(crate::core::events::DomainEvent::ThreadGoalCleared {
            thread_id: thread_id.to_string(),
        });
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
