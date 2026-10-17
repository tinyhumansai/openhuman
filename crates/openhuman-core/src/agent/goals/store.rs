//! Workspace-path adapter onto tinyagents' authoritative goal store.
//!
//! **Crate-backed.** The per-thread goal now lives in the vendored `tinyagents`
//! crate's `graph::goals` KV store
//! (the `graph.goals` namespace of the agent's key-value store), not
//! the legacy `<workspace>/thread_goals/` file-JSON tree. This module is a thin
//! adapter for the harness turn loop, agent tools, and post-turn accounting. It forwards each operation to the crate
//! store, converting the crate
//! [`CrateThreadGoal`](tinyagents_graph::goals::types::ThreadGoal) back to the
//! local [`ThreadGoal`] and its `TinyAgentsError` to a `String`.
//!
//! The crate store owns the concurrency (per-thread async locks) and the goal
//! business logic (objective-change reset, budget re-open, status transitions,
//! suppression compare-and-set, and stale-goal-id accounting guards).

use std::path::Path;
use std::sync::Arc;

use tinyagents_graph::goals::store as crate_store;
use tinyagents_graph::goals::ThreadGoal;
use tinyagents_harness::store::Store;

/// The store thread goals live in: the current agent's key-value store in the
/// storage backend ([`crate::agent::session_store::stores_for`]).
pub(crate) fn goals_store(workspace_dir: &Path) -> Arc<dyn Store> {
    crate::agent::session_store::stores_or_ephemeral(workspace_dir).kv
}

/// Set (create or replace) the thread's goal. A changed objective mints a fresh
/// goal and resets counters; an unchanged objective preserves counters and
/// re-opens the goal (staying `BudgetLimited` if still over budget).
pub async fn set(
    workspace_dir: &Path,
    thread_id: &str,
    objective: &str,
    token_budget: Option<u64>,
) -> Result<ThreadGoal, String> {
    let store = goals_store(workspace_dir);
    let goal = crate_store::set(&store, thread_id, objective, token_budget)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(
        thread_id = %goal.thread_id,
        goal_id = %goal.goal_id,
        "[thread_goals] set objective ({} chars), budget={:?}",
        goal.objective.chars().count(),
        goal.token_budget
    );
    Ok(goal)
}

/// Set the goal **only if the thread has none yet**. `Some(goal)` when created,
/// `None` when a goal already existed (left untouched).
pub async fn set_if_absent(
    workspace_dir: &Path,
    thread_id: &str,
    objective: &str,
    token_budget: Option<u64>,
) -> Result<Option<ThreadGoal>, String> {
    let store = goals_store(workspace_dir);
    let goal = crate_store::set_if_absent(&store, thread_id, objective, token_budget)
        .await
        .map_err(|e| e.to_string())?;
    Ok(goal)
}

/// The thread's current goal, or `None`.
pub async fn get(workspace_dir: &Path, thread_id: &str) -> Result<Option<ThreadGoal>, String> {
    let store = goals_store(workspace_dir);
    let goal = crate_store::get(&store, thread_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(goal)
}

/// Delete the thread's goal. Returns whether a goal was present.
pub async fn clear(workspace_dir: &Path, thread_id: &str) -> Result<bool, String> {
    let store = goals_store(workspace_dir);
    let existed = crate_store::clear(&store, thread_id)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(thread_id = %thread_id.trim(), existed, "[thread_goals] clear");
    Ok(existed)
}

/// Mark the goal `Complete` (model-driven success).
pub async fn complete(workspace_dir: &Path, thread_id: &str) -> Result<ThreadGoal, String> {
    let store = goals_store(workspace_dir);
    let goal = crate_store::complete(&store, thread_id)
        .await
        .map_err(|e| e.to_string())?;
    tracing::info!(thread_id = %goal.thread_id, goal_id = %goal.goal_id, "[thread_goals] complete");
    Ok(goal)
}

/// Mark the goal `Paused`.
pub async fn pause(workspace_dir: &Path, thread_id: &str) -> Result<ThreadGoal, String> {
    let store = goals_store(workspace_dir);
    let goal = crate_store::pause(&store, thread_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(goal)
}

/// Resume a `Paused` goal back to `Active`.
pub async fn resume(workspace_dir: &Path, thread_id: &str) -> Result<ThreadGoal, String> {
    let store = goals_store(workspace_dir);
    let goal = crate_store::resume(&store, thread_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(goal)
}

/// Account token + time usage against the goal, applying the budget constraint.
/// The delta is ignored when `expected_goal_id` no longer matches the current
/// goal (stale-write guard). Returns the goal after the (possibly skipped)
/// update, or `None` when the thread has no goal.
pub async fn account_usage(
    workspace_dir: &Path,
    thread_id: &str,
    expected_goal_id: &str,
    token_delta: u64,
    secs_delta: u64,
) -> Result<Option<ThreadGoal>, String> {
    let store = goals_store(workspace_dir);
    let goal =
        crate_store::account_usage(&store, thread_id, expected_goal_id, token_delta, secs_delta)
            .await
            .map_err(|e| e.to_string())?;
    Ok(goal)
}
