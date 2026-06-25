//! Business logic for the thread-goal domain — thin handlers over
//! [`super::store`], each returning an [`RpcOutcome`] so the RPC layer and CLI
//! share a uniform shape with logs. Mutating ops emit `thread/goal/updated` (or
//! `thread/goal/cleared`) domain events so the desktop UI can live-update.

use std::path::Path;

use serde::Serialize;

use super::store;
use super::types::ThreadGoal;
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::rpc::RpcOutcome;

/// Envelope returned by reads/clears where the goal may be absent.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GoalEnvelope {
    /// The current goal, or `null` when the thread has none.
    pub goal: Option<ThreadGoal>,
}

/// Result of a clear operation.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClearResult {
    /// Whether a goal existed and was removed.
    pub removed: bool,
}

/// Publish a `thread/goal/updated` event for live UI sync (best-effort).
fn emit_updated(goal: &ThreadGoal) {
    publish_global(DomainEvent::ThreadGoalUpdated {
        thread_id: goal.thread_id.clone(),
        goal_id: goal.goal_id.clone(),
        status: goal.status.as_str().to_string(),
    });
}

/// Get the goal for a thread (or `null`).
pub async fn get(
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<RpcOutcome<GoalEnvelope>, String> {
    log::debug!("[thread_goals] rpc=get thread_id={thread_id}");
    let goal = store::get(workspace_dir, thread_id).await?;
    Ok(RpcOutcome::new(GoalEnvelope { goal }, vec![]))
}

/// Create or replace the goal for a thread.
pub async fn set(
    workspace_dir: &Path,
    thread_id: &str,
    objective: &str,
    token_budget: Option<u64>,
) -> Result<RpcOutcome<GoalEnvelope>, String> {
    log::debug!("[thread_goals] rpc=set thread_id={thread_id}");
    let goal = store::set(workspace_dir, thread_id, objective, token_budget).await?;
    emit_updated(&goal);
    Ok(RpcOutcome::single_log(
        GoalEnvelope {
            goal: Some(goal.clone()),
        },
        format!(
            "set thread goal {} ({})",
            goal.goal_id,
            goal.status.as_str()
        ),
    ))
}

/// Mark the goal complete.
pub async fn complete(
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<RpcOutcome<GoalEnvelope>, String> {
    log::debug!("[thread_goals] rpc=complete thread_id={thread_id}");
    let goal = store::complete(workspace_dir, thread_id).await?;
    emit_updated(&goal);
    Ok(RpcOutcome::single_log(
        GoalEnvelope {
            goal: Some(goal.clone()),
        },
        format!("completed thread goal {}", goal.goal_id),
    ))
}

/// Pause an active goal.
pub async fn pause(
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<RpcOutcome<GoalEnvelope>, String> {
    log::debug!("[thread_goals] rpc=pause thread_id={thread_id}");
    let goal = store::pause(workspace_dir, thread_id).await?;
    emit_updated(&goal);
    Ok(RpcOutcome::single_log(
        GoalEnvelope {
            goal: Some(goal.clone()),
        },
        format!("paused thread goal {}", goal.goal_id),
    ))
}

/// Resume a paused goal.
pub async fn resume(
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<RpcOutcome<GoalEnvelope>, String> {
    log::debug!("[thread_goals] rpc=resume thread_id={thread_id}");
    let goal = store::resume(workspace_dir, thread_id).await?;
    emit_updated(&goal);
    Ok(RpcOutcome::single_log(
        GoalEnvelope {
            goal: Some(goal.clone()),
        },
        format!("resumed thread goal {}", goal.goal_id),
    ))
}

/// Clear (delete) the goal for a thread.
pub async fn clear(
    workspace_dir: &Path,
    thread_id: &str,
) -> Result<RpcOutcome<ClearResult>, String> {
    log::debug!("[thread_goals] rpc=clear thread_id={thread_id}");
    let removed = store::clear(workspace_dir, thread_id).await?;
    if removed {
        publish_global(DomainEvent::ThreadGoalCleared {
            thread_id: thread_id.to_string(),
        });
    }
    Ok(RpcOutcome::single_log(
        ClearResult { removed },
        format!("cleared thread goal (removed={removed})"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn set_get_complete_clear_flow() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();

        // Empty to start.
        let got = get(dir, "t").await.unwrap();
        assert!(got.value.goal.is_none());

        // Set.
        let set_out = set(dir, "t", "ship it", Some(1000)).await.unwrap();
        let goal = set_out.value.goal.unwrap();
        assert_eq!(goal.objective, "ship it");

        // Complete.
        let done = complete(dir, "t").await.unwrap();
        assert_eq!(done.value.goal.unwrap().status.as_str(), "complete");

        // Clear.
        let cleared = clear(dir, "t").await.unwrap();
        assert!(cleared.value.removed);
        assert!(get(dir, "t").await.unwrap().value.goal.is_none());
    }
}
