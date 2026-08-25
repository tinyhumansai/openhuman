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

use tinyagents::graph::goals::budget as crate_budget;
use tinyagents::graph::goals::{BudgetVerdict, GoalBudgetGuard};

use super::migration::goals_store;
use super::store;
use super::{ThreadGoal, ThreadGoalStatus};
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::stop_hooks::{StopDecision, StopHook, TurnState};
use crate::openhuman::agent::tinyagents::thread_context::current_thread_id;

/// Load the goal for the ambient chat thread, if any. Returns `None` outside a
/// thread scope (CLI / background paths) or when the thread has no goal.
pub async fn load_for_current_thread(workspace_dir: &Path) -> Option<ThreadGoal> {
    let thread_id = current_thread_id()?;
    match store::get(workspace_dir, &thread_id).await {
        Ok(goal) => goal,
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] load_for_current_thread failed");
            None
        }
    }
}

/// Reactivate a paused goal for the ambient thread (thread-resume semantics).
/// Returns the updated goal, or `None` outside a thread scope / when absent.
/// Best-effort: a failure is logged and surfaced as `Ok(None)`-style `None`.
pub async fn resume_for_current_thread(workspace_dir: &Path) -> Option<Option<ThreadGoal>> {
    let thread_id = current_thread_id()?;
    match store::resume(workspace_dir, &thread_id).await {
        Ok(goal) => {
            if goal.status.is_active() {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: goal.thread_id.clone(),
                    goal_id: goal.goal_id.clone(),
                    status: goal.status.as_str().to_string(),
                });
            }
            Some(Some(goal))
        }
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] resume_for_current_thread failed");
            None
        }
    }
}

/// Pause the active goal for the ambient thread (interrupt/abort semantics).
/// Best-effort; safe to call when there is no goal or no thread scope.
pub async fn pause_for_current_thread(workspace_dir: &Path) {
    let Some(thread_id) = current_thread_id() else {
        return;
    };
    match store::pause(workspace_dir, &thread_id).await {
        Ok(goal) => {
            if matches!(goal.status, ThreadGoalStatus::Paused) {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: goal.thread_id.clone(),
                    goal_id: goal.goal_id.clone(),
                    status: goal.status.as_str().to_string(),
                });
            }
        }
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] pause_for_current_thread failed");
        }
    }
}

/// Mark the active goal for the ambient thread `Complete` (the originating
/// task settled successfully). Best-effort; safe to call when there is no goal
/// or no thread scope. Emits `ThreadGoalUpdated` so the UI chip refreshes.
///
/// This is the lifecycle counterpart the pause/resume pair was missing: without
/// a settle, a goal a finished task left behind stays `Active` and is
/// re-injected as an `[active_goal]` block on every later turn — including
/// unrelated chat (#1725). A caller that owns a task's lifecycle calls this
/// when the task reaches a terminal, satisfied state so the goal can't linger.
pub async fn complete_for_current_thread(workspace_dir: &Path) {
    let Some(thread_id) = current_thread_id() else {
        return;
    };
    match store::complete(workspace_dir, &thread_id).await {
        Ok(goal) => {
            if matches!(goal.status, ThreadGoalStatus::Complete) {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: goal.thread_id.clone(),
                    goal_id: goal.goal_id.clone(),
                    status: goal.status.as_str().to_string(),
                });
            }
        }
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] complete_for_current_thread failed");
        }
    }
}

/// Delete the goal row for the ambient thread entirely (the originating task was
/// abandoned / superseded, and no completion contract should persist).
/// Best-effort; safe to call when there is no goal or no thread scope.
///
/// Clearing removes the row rather than moving it to a terminal status, so a
/// later turn loads `None` and injects no `[active_goal]` block at all — the
/// strongest guarantee that a stale objective cannot leak forward (#1725).
pub async fn clear_for_current_thread(workspace_dir: &Path) {
    let Some(thread_id) = current_thread_id() else {
        return;
    };
    match store::clear(workspace_dir, &thread_id).await {
        Ok(_existed) => {}
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] clear_for_current_thread failed");
        }
    }
}

/// The per-turn token total used for budget accounting (prompt + completion).
fn turn_tokens(input: u64, output: u64) -> u64 {
    crate_budget::turn_tokens(input, output)
}

/// Whether the current turn is an autonomous goal-continuation (vs. a
/// user-initiated turn). Used so a continuation doesn't clear its own one-shot
/// suppression flag.
fn is_goal_continuation_turn() -> bool {
    matches!(
        crate::openhuman::agent::turn_origin::current(),
        Some(
            crate::openhuman::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
                source:
                    crate::openhuman::agent::turn_origin::TrustedAutomationSource::GoalContinuation,
                ..
            }
        )
    )
}

/// Account a finished turn's usage against the ambient thread's goal.
///
/// The accounting rules are the crate's
/// ([`crate_budget::account_turn`](tinyagents::graph::goals::account_turn)):
/// only **active** goals are charged, so a paused/complete/budget-limited goal
/// doesn't accrue usage from incidental chat, and a user-initiated turn clears
/// the one-shot continuation suppression (a continuation turn must not clear
/// its own, see [`super::continuation`]).
///
/// What is OpenHuman's here: reading the ambient thread from the turn scope,
/// classifying the turn as user-initiated vs. continuation from its origin, and
/// emitting `ThreadGoalUpdated` when the status changes (e.g. →
/// `budget_limited`) so the UI chip refreshes. Best-effort throughout: a
/// failure is logged and swallowed so accounting never fails a user turn.
pub async fn account_turn_against_goal(workspace_dir: &Path, input: u64, output: u64, secs: u64) {
    let Some(thread_id) = current_thread_id() else {
        return;
    };
    let prev_status = match store::get(workspace_dir, &thread_id).await {
        Ok(Some(goal)) => goal.status,
        Ok(None) => return,
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] account get failed");
            return;
        }
    };

    let store = goals_store(workspace_dir);
    let user_initiated = !is_goal_continuation_turn();
    match crate_budget::account_turn(&store, &thread_id, input, output, secs, user_initiated).await
    {
        Ok(Some(updated)) => {
            tracing::debug!(
                thread_id = %thread_id,
                goal_id = %updated.goal_id,
                tokens_used = updated.tokens_used,
                status = updated.status.as_str(),
                "[thread_goals] accounted turn usage (+{} tok, +{secs}s)",
                turn_tokens(input, output)
            );
            if updated.status != prev_status {
                BUS.publish(DomainEvent::ThreadGoalUpdated {
                    thread_id: updated.thread_id.clone(),
                    goal_id: updated.goal_id.clone(),
                    status: updated.status.as_str().to_string(),
                });
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] account_turn failed");
        }
    }
}

/// Mid-turn stop hook that halts an in-flight turn once an **active** goal's
/// running usage (already-accounted tokens from prior turns + this turn's
/// tokens so far) would meet or exceed its budget.
///
/// The decision is the crate's
/// [`GoalBudgetGuard`](tinyagents::graph::goals::GoalBudgetGuard); this is the
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
impl StopHook for GoalBudgetStopHook {
    fn name(&self) -> &str {
        "thread_goal_budget"
    }

    async fn check(&self, ctx: &TurnState<'_>) -> StopDecision {
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
mod tests {
    use super::*;
    use crate::openhuman::agent::cost::TurnCost;
    use crate::openhuman::inference::provider::UsageInfo;

    fn cost_with_tokens(input: u64, output: u64) -> TurnCost {
        let mut tc = TurnCost::new();
        tc.add_call(
            "agentic-v1",
            &UsageInfo {
                input_tokens: input,
                output_tokens: output,
                ..Default::default()
            },
        );
        tc
    }

    #[tokio::test]
    async fn account_turn_charges_active_goal_and_trips_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-acct", async {
            store::set(&dir, "t-acct", "obj", Some(100)).await.unwrap();
            account_turn_against_goal(&dir, 80, 40, 3).await; // 120 >= 100
            let g = store::get(&dir, "t-acct").await.unwrap().unwrap();
            assert_eq!(g.tokens_used, 120);
            assert_eq!(g.status, ThreadGoalStatus::BudgetLimited);
        })
        .await;
    }

    #[tokio::test]
    async fn account_turn_skips_non_active_goal() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-paused", async {
            store::set(&dir, "t-paused", "obj", Some(1000))
                .await
                .unwrap();
            store::pause(&dir, "t-paused").await.unwrap();
            account_turn_against_goal(&dir, 500, 500, 1).await;
            let g = store::get(&dir, "t-paused").await.unwrap().unwrap();
            assert_eq!(g.tokens_used, 0, "paused goal must not accrue usage");
        })
        .await;
    }

    #[tokio::test]
    async fn complete_for_current_thread_settles_active_goal() {
        // A goal an originating task left behind is settled to `Complete` so a
        // later turn stops re-injecting its `[active_goal]` block (#1725).
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-complete", async {
            store::set(&dir, "t-complete", "obj", Some(1000))
                .await
                .unwrap();
            complete_for_current_thread(&dir).await;
            let g = store::get(&dir, "t-complete").await.unwrap().unwrap();
            assert_eq!(
                g.status,
                ThreadGoalStatus::Complete,
                "an originating task's settle must mark the goal Complete, not leave it Active"
            );
            assert!(
                !g.status.is_active(),
                "a completed goal must not stay active (it would keep steering unrelated turns)"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn clear_for_current_thread_removes_the_goal_row() {
        // Clearing deletes the row entirely, so a later turn loads `None` and
        // injects no goal block at all — the strongest anti-leak guarantee.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-clear", async {
            store::set(&dir, "t-clear", "obj", Some(1000))
                .await
                .unwrap();
            clear_for_current_thread(&dir).await;
            let g = store::get(&dir, "t-clear").await.unwrap();
            assert!(
                g.is_none(),
                "clearing must delete the goal row so no [active_goal] can leak forward"
            );
        })
        .await;
    }

    #[tokio::test]
    async fn complete_and_clear_are_safe_without_a_goal_or_thread() {
        // Best-effort contract: both are no-ops (never panic) when there is no
        // goal for the thread, and when called outside any thread scope.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id("t-empty", async {
            complete_for_current_thread(&dir).await;
            clear_for_current_thread(&dir).await;
            assert!(store::get(&dir, "t-empty").await.unwrap().is_none());
        })
        .await;
        // No ambient thread scope at all.
        complete_for_current_thread(&dir).await;
        clear_for_current_thread(&dir).await;
    }

    #[tokio::test]
    async fn account_turn_clears_suppression_without_losing_usage() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::agent::tinyagents::thread_context::with_thread_id(
            "t-suppressed",
            async {
                let goal = store::set(&dir, "t-suppressed", "obj", Some(1000))
                    .await
                    .unwrap();
                store::set_continuation_suppressed_if(&dir, "t-suppressed", &goal.goal_id, true)
                    .await
                    .unwrap();

                account_turn_against_goal(&dir, 80, 40, 3).await;

                let updated = store::get(&dir, "t-suppressed").await.unwrap().unwrap();
                assert_eq!(updated.goal_id, goal.goal_id);
                assert!(!updated.continuation_suppressed);
                assert_eq!(updated.tokens_used, 120);
                assert_eq!(updated.time_used_seconds, 3);
            },
        )
        .await;
    }

    #[tokio::test]
    async fn budget_stop_hook_fires_on_crossing() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let goal = store::set(&dir, "t-hook", "obj", Some(1000)).await.unwrap();
        // 600 already used in a prior turn.
        store::account_usage(&dir, "t-hook", &goal.goal_id, 600, 0)
            .await
            .unwrap();
        let goal = store::get(&dir, "t-hook").await.unwrap().unwrap();
        let hook = GoalBudgetStopHook::for_goal(&dir, &goal).expect("budgeted active goal");

        // This turn so far: 300 in + 200 out = 500. 600 + 500 = 1100 >= 1000.
        let cost = cost_with_tokens(300, 200);
        let ctx = TurnState {
            iteration: 2,
            max_iterations: 10,
            cost: &cost,
            model: "agentic-v1",
        };
        assert!(matches!(hook.check(&ctx).await, StopDecision::Stop { .. }));

        // Under the cap continues.
        let small = cost_with_tokens(100, 100); // 600 + 200 = 800 < 1000
        let ctx2 = TurnState {
            iteration: 1,
            max_iterations: 10,
            cost: &small,
            model: "agentic-v1",
        };
        assert!(matches!(hook.check(&ctx2).await, StopDecision::Continue));
    }

    #[tokio::test]
    async fn no_hook_without_budget_or_when_inactive() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        let no_budget = store::set(&dir, "a", "obj", None).await.unwrap();
        assert!(GoalBudgetStopHook::for_goal(&dir, &no_budget).is_none());
        store::set(&dir, "b", "obj", Some(100)).await.unwrap();
        store::pause(&dir, "b").await.unwrap();
        let paused = store::get(&dir, "b").await.unwrap().unwrap();
        assert!(GoalBudgetStopHook::for_goal(&dir, &paused).is_none());
    }
}
