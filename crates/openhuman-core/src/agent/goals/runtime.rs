//! OpenHuman runtime adapters for tinyagents thread-goal accounting and the
//! host-specific mid-turn budget stop hook.
//!
//! These are the pieces that make a stored goal actually steer the agent
//! (Codex parity):
//!
//! - [`account_turn_against_goal`] folds a completed turn's token + time usage
//!   into the active goal, flipping it to `budget_limited` when the cap is
//!   crossed.
//! - [`GoalBudgetStopHook`] votes to stop an in-flight turn as soon as an
//!   *active* goal's running usage would exceed its budget. #4469 item 1: the
//!   stop is a graceful *pause*, not an instantaneous abort — the vote fires in
//!   the stop-hook middleware's `after_model`, and the harness drains the pause
//!   at the **top of the next iteration**, so the tool round for the model call
//!   that tripped the budget still runs and the turn's wrap-up summary may spend
//!   one more model call before the partial transcript is returned. It bounds
//!   an autonomous run to a small, deterministic overshoot past the ceiling
//!   rather than a hard cut at the exact accounting point.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use tinyagents_graph::goals::budget as crate_budget;
use tinyagents_graph::goals::{BudgetVerdict, GoalBudgetGuard, ThreadGoal, ThreadGoalStatus};

use super::store::{self, goals_store};
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use tinyagents_harness::middleware::{StopDecision, StopHook, TurnState};

/// Load the goal for an explicitly supplied chat thread.
pub async fn load_for_thread(workspace_dir: &Path, thread_id: Option<&str>) -> Option<ThreadGoal> {
    let thread_id = normalized_thread(thread_id)?;
    match store::get(workspace_dir, &thread_id).await {
        Ok(goal) => goal,
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] load_for_thread failed");
            None
        }
    }
}

/// Reactivate a paused goal for an explicit thread (thread-resume semantics).
/// Best-effort: a failure is logged and surfaced as `Ok(None)`-style `None`.
pub async fn resume_for_thread(
    workspace_dir: &Path,
    thread_id: Option<&str>,
) -> Option<Option<ThreadGoal>> {
    let thread_id = normalized_thread(thread_id)?;
    match store::resume(workspace_dir, &thread_id).await {
        Ok(goal) => {
            if goal.status.is_active() {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: goal.thread_id.clone(),
                    goal_id: goal.goal_id.clone(),
                    status: goal.status.as_str().to_string(),
                    goal: Some(super::goal_to_value(&goal)),
                });
            }
            Some(Some(goal))
        }
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] resume_for_thread failed");
            None
        }
    }
}

/// Mark the active goal for an explicit thread `Complete` (the originating
/// task settled successfully). Best-effort; safe to call when there is no goal
/// or no thread id. Emits `ThreadGoalUpdated` so the UI chip refreshes.
///
/// This is the lifecycle counterpart the pause/resume pair was missing: without
/// a settle, a goal a finished task left behind stays `Active` and is
/// re-injected as an `[active_goal]` block on every later turn — including
/// unrelated chat (#1725). A caller that owns a task's lifecycle calls this
/// when the task reaches a terminal, satisfied state so the goal can't linger.
pub async fn complete_for_thread(workspace_dir: &Path, thread_id: Option<&str>) {
    let Some(thread_id) = normalized_thread(thread_id) else {
        return;
    };
    match store::complete(workspace_dir, &thread_id).await {
        Ok(goal) => {
            if matches!(goal.status, ThreadGoalStatus::Complete) {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: goal.thread_id.clone(),
                    goal_id: goal.goal_id.clone(),
                    status: goal.status.as_str().to_string(),
                    goal: Some(super::goal_to_value(&goal)),
                });
            }
        }
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] complete_for_thread failed");
        }
    }
}

/// Delete the goal row for an explicit thread entirely (the originating task was
/// abandoned / superseded, and no completion contract should persist).
/// Best-effort; safe to call when there is no goal or thread id.
///
/// Clearing removes the row rather than moving it to a terminal status, so a
/// later turn loads `None` and injects no `[active_goal]` block at all — the
/// strongest guarantee that a stale objective cannot leak forward (#1725).
pub async fn clear_for_thread(workspace_dir: &Path, thread_id: Option<&str>) {
    let Some(thread_id) = normalized_thread(thread_id) else {
        return;
    };
    match store::clear(workspace_dir, &thread_id).await {
        Ok(existed) => {
            if existed {
                BUS.publish(DomainEvent::ThreadGoalCleared {
                    thread_id: thread_id.clone(),
                });
            }
        }
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] clear_for_thread failed");
        }
    }
}

/// The per-turn token total used for budget accounting (prompt + completion).
fn turn_tokens(input: u64, output: u64) -> u64 {
    crate_budget::turn_tokens(input, output)
}

/// Account a finished turn's usage against an explicit thread's goal.
///
/// The accounting rules are the crate's
/// ([`crate_budget::account_turn`](tinyagents_graph::goals::account_turn)):
/// only **active** goals are charged, so a paused/complete/budget-limited goal
/// doesn't accrue usage from incidental chat. OpenHuman runs no autonomous
/// goal continuation, so every accounted turn is user-initiated.
///
/// What is OpenHuman's here: receiving the owned thread from the turn and
/// emitting `ThreadGoalUpdated` when the status changes (e.g. →
/// `budget_limited`) so the UI chip refreshes. Best-effort throughout: a
/// failure is logged and swallowed so accounting never fails a user turn.
pub async fn account_turn_against_goal(
    workspace_dir: &Path,
    thread_id: Option<&str>,
    input: u64,
    output: u64,
    secs: u64,
) {
    let Some(thread_id) = normalized_thread(thread_id) else {
        return;
    };
    let prev = match store::get(workspace_dir, &thread_id).await {
        Ok(Some(goal)) => goal,
        Ok(None) => return,
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] account get failed");
            return;
        }
    };
    let prev_status = prev.status;
    let prev_tokens_used = prev.tokens_used;

    let store = goals_store(workspace_dir);
    match crate_budget::account_turn(&store, &thread_id, input, output, secs, true).await {
        Ok(Some(updated)) => {
            tracing::debug!(
                thread_id = %thread_id,
                goal_id = %updated.goal_id,
                tokens_used = updated.tokens_used,
                status = updated.status.as_str(),
                "[thread_goals] accounted turn usage (+{} tok, +{secs}s)",
                turn_tokens(input, output)
            );
            // Publish on any status transition, or — for a live budget
            // display — when accumulated usage has moved by at least 5% of
            // the configured budget since the last publish. Without the
            // throttle every single turn's accounting would emit a socket
            // event; the threshold keeps the UI's budget meter live without
            // flooding the bus on chatty threads.
            let status_changed = updated.status != prev_status;
            let budget_moved = updated
                .token_budget
                .filter(|b| *b > 0)
                .is_some_and(|budget| {
                    let delta = updated.tokens_used.saturating_sub(prev_tokens_used);
                    // 5% of budget, at least 1 token so a tiny budget still reports.
                    let threshold = (budget / 20).max(1);
                    delta >= threshold
                });
            if status_changed || budget_moved {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: updated.thread_id.clone(),
                    goal_id: updated.goal_id.clone(),
                    status: updated.status.as_str().to_string(),
                    goal: Some(super::goal_to_value(&updated)),
                });
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] account_turn failed");
        }
    }
}

fn normalized_thread(thread_id: Option<&str>) -> Option<String> {
    thread_id
        .map(str::trim)
        .filter(|thread_id| !thread_id.is_empty())
        .map(ToOwned::to_owned)
}

/// Mid-turn stop hook that halts an in-flight turn once an **active** goal's
/// running usage (already-accounted tokens from prior turns + this turn's
/// tokens so far) would meet or exceed its budget.
///
/// The decision is the crate's
/// [`GoalBudgetGuard`](tinyagents_graph::goals::GoalBudgetGuard); this is the
/// adapter that votes it into OpenHuman's [`StopHook`] chain. #4469 item 1: the
/// stop is a graceful *pause*, not an instantaneous abort — the vote fires in
/// the stop-hook middleware's `after_model`, and the harness drains the pause
/// at the **top of the next iteration**, so the tool round for the model call
/// that tripped the budget still runs and the turn's wrap-up summary may spend
/// one more model call before the partial transcript is returned. It bounds an
/// autonomous run to a small, deterministic overshoot past the ceiling rather
/// than a hard cut at the exact accounting point.
///
/// The guard only arms for a goal that is `Active` with a configured budget,
/// and stands down if that goal is completed, replaced, or paused mid-turn —
/// once a goal is `budget_limited`/`paused`/`complete` the user can still chat
/// freely (the injected context steers the model to summarise), so a
/// user-present turn is never hard-stopped by a budget that is no longer live.
#[derive(Debug, Clone)]
pub struct GoalBudgetStopHook {
    workspace_dir: PathBuf,
    guard: GoalBudgetGuard,
}

impl GoalBudgetStopHook {
    /// Build a hook for `goal` if it's active and has a budget; `None` otherwise.
    pub fn for_goal(workspace_dir: &Path, goal: &ThreadGoal) -> Option<Self> {
        Some(Self {
            workspace_dir: workspace_dir.to_path_buf(),
            guard: GoalBudgetGuard::for_goal(goal)?,
        })
    }
}

#[async_trait]
impl StopHook<tinyagents_harness::cost::TurnCost> for GoalBudgetStopHook {
    fn name(&self) -> &str {
        "thread_goal_budget"
    }

    async fn check(&self, ctx: &TurnState<'_, tinyagents_harness::cost::TurnCost>) -> StopDecision {
        let store = goals_store(&self.workspace_dir);
        let in_flight = turn_tokens(ctx.cost.input_tokens, ctx.cost.output_tokens);
        match self.guard.check(&store, in_flight).await {
            Ok(BudgetVerdict::Stop { reason }) => StopDecision::Stop { reason },
            Ok(BudgetVerdict::Continue) => StopDecision::Continue,
            Err(e) => {
                // An unreadable goal is not grounds for killing a live turn.
                tracing::debug!(error = %e, "[thread_goals] budget check failed; continuing");
                StopDecision::Continue
            }
        }
    }
}

#[cfg(test)]
#[path = "runtime_tests.rs"]
mod tests;
