//! Harness-level runtime for the thread goal: per-turn context injection,
//! token-budget accounting, and the mid-turn budget stop hook.
//!
//! These are the pieces that make a stored goal actually steer the agent
//! (Codex parity):
//!
//! - [`active_goal_context_block`] renders a compact `[active_goal]` block that
//!   the turn loop prepends to the user message **fresh each turn** (never the
//!   cached system-prompt prefix), so the objective stays visible and the model
//!   sees live budget/status.
//! - [`account_turn_against_goal`] folds a completed turn's token + time usage
//!   into the active goal, flipping it to `budget_limited` when the cap is
//!   crossed.
//! - [`GoalBudgetStopHook`] hard-stops an in-flight turn the moment an *active*
//!   goal's running usage would exceed its budget, so an autonomous run can't
//!   blow past the ceiling between accounting points.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::store;
use super::types::{ThreadGoal, ThreadGoalStatus};
use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::agent::stop_hooks::{StopDecision, StopHook, TurnState};
use crate::openhuman::inference::provider::thread_context::current_thread_id;

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
                publish_global(DomainEvent::ThreadGoalUpdated {
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
                publish_global(DomainEvent::ThreadGoalUpdated {
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

/// Render the per-turn `[active_goal]` context block for `goal`, or `None` when
/// the goal is in a state that needs no steering text.
///
/// The block is intentionally tiny and source-attributed so it reads as harness
/// state, not user instruction.
pub fn active_goal_context_block(goal: &ThreadGoal) -> Option<String> {
    let directive = match goal.status {
        ThreadGoalStatus::Active => {
            "Keep working toward this goal. When evidence confirms it's done, call \
             goal_complete; if the objective has changed, update it with goal_set."
        }
        ThreadGoalStatus::BudgetLimited => {
            "This goal has reached its token budget. Stop substantive work: summarise \
             progress and blockers, and name the next useful step. Do not continue \
             until the user raises the budget or clears the goal."
        }
        // A paused goal isn't being worked right now; a completed goal needs no
        // steering. Surfacing them would only add noise to the turn.
        ThreadGoalStatus::Paused | ThreadGoalStatus::Complete => return None,
    };
    let budget = match (goal.token_budget, goal.budget_remaining()) {
        (Some(b), Some(rem)) => format!(
            "\nbudget: {} used / {b} ({rem} remaining)",
            goal.tokens_used
        ),
        _ => String::new(),
    };
    Some(format!(
        "[active_goal]\nstatus: {}\nobjective: {}{}\n{}\n[/active_goal]\n\n",
        goal.status.as_str(),
        goal.objective,
        budget,
        directive
    ))
}

/// The per-turn token total used for budget accounting (prompt + completion).
fn turn_tokens(input: u64, output: u64) -> u64 {
    input.saturating_add(output)
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
/// Only **active** goals are charged (a paused/complete/budget-limited goal
/// doesn't accrue usage from incidental chat). Best-effort: a failure is logged
/// and swallowed so accounting never fails a user turn. Emits
/// `ThreadGoalUpdated` when the status changes (e.g. → `budget_limited`) so the
/// UI chip refreshes.
pub async fn account_turn_against_goal(workspace_dir: &Path, input: u64, output: u64, secs: u64) {
    let Some(thread_id) = current_thread_id() else {
        return;
    };
    let goal = match store::get(workspace_dir, &thread_id).await {
        Ok(Some(g)) => g,
        Ok(None) => return,
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] account get failed");
            return;
        }
    };
    if !goal.status.is_active() {
        return;
    }
    // Reset the one-shot continuation suppression on user-initiated activity: a
    // real turn in this thread means the user re-engaged, so a future idle
    // period may auto-continue again. The continuation turn itself runs under a
    // GoalContinuation origin and must NOT clear its own suppression.
    if goal.continuation_suppressed && !is_goal_continuation_turn() {
        if let Err(e) = store::set_continuation_suppressed(workspace_dir, &thread_id, false).await {
            tracing::debug!(
                thread_id = %thread_id,
                error = %e,
                "[thread_goals] failed to clear continuation suppression"
            );
        }
    }
    let delta = turn_tokens(input, output);
    if delta == 0 && secs == 0 {
        return;
    }
    let prev_status = goal.status;
    match store::account_usage(workspace_dir, &thread_id, &goal.goal_id, delta, secs).await {
        Ok(Some(updated)) => {
            tracing::debug!(
                thread_id = %thread_id,
                goal_id = %updated.goal_id,
                tokens_used = updated.tokens_used,
                status = updated.status.as_str(),
                "[thread_goals] accounted turn usage (+{delta} tok, +{secs}s)"
            );
            if updated.status != prev_status {
                publish_global(DomainEvent::ThreadGoalUpdated {
                    thread_id: updated.thread_id.clone(),
                    goal_id: updated.goal_id.clone(),
                    status: updated.status.as_str().to_string(),
                });
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::debug!(thread_id = %thread_id, error = %e, "[thread_goals] account_usage failed");
        }
    }
}

/// Mid-turn stop hook that halts an in-flight turn once an **active** goal's
/// running usage (already-accounted tokens from prior turns + this turn's
/// tokens so far) would meet or exceed its budget.
///
/// It only fires for goals that are still `Active` with a configured budget —
/// once a goal is `budget_limited`/`paused`/`complete` the user can still chat
/// freely (the injected context steers the model to summarise), so we never
/// hard-stop a user-present turn that isn't actively burning a live budget.
#[derive(Debug, Clone)]
pub struct GoalBudgetStopHook {
    workspace_dir: PathBuf,
    thread_id: String,
    /// The goal version this hook was armed for. Stops enforcing if the goal is
    /// replaced mid-turn (a new objective mints a new id).
    goal_id: String,
    budget: u64,
}

impl GoalBudgetStopHook {
    /// Build a hook for `goal` if it's active and has a budget; `None` otherwise.
    pub fn for_goal(workspace_dir: &Path, goal: &ThreadGoal) -> Option<Self> {
        if !goal.status.is_active() {
            return None;
        }
        let budget = goal.token_budget?;
        Some(Self {
            workspace_dir: workspace_dir.to_path_buf(),
            thread_id: goal.thread_id.clone(),
            goal_id: goal.goal_id.clone(),
            budget,
        })
    }
}

#[async_trait]
impl StopHook for GoalBudgetStopHook {
    fn name(&self) -> &str {
        "thread_goal_budget"
    }

    async fn check(&self, ctx: &TurnState<'_>) -> StopDecision {
        // Read the goal's already-accounted usage (prior turns). If it's gone,
        // replaced, or no longer active, stop enforcing.
        let goal = match store::get(&self.workspace_dir, &self.thread_id).await {
            Ok(Some(g)) => g,
            _ => return StopDecision::Continue,
        };
        if goal.goal_id != self.goal_id || !goal.status.is_active() {
            return StopDecision::Continue;
        }
        let projected = goal
            .tokens_used
            .saturating_add(turn_tokens(ctx.cost.input_tokens, ctx.cost.output_tokens));
        if projected >= self.budget {
            StopDecision::Stop {
                reason: format!(
                    "thread goal budget reached: {projected} tokens >= {} budget — stopping to summarise progress",
                    self.budget
                ),
            }
        } else {
            StopDecision::Continue
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

    #[test]
    fn active_block_includes_objective_and_budget() {
        let goal = ThreadGoal {
            thread_id: "t".into(),
            goal_id: "g".into(),
            objective: "ship the feature".into(),
            status: ThreadGoalStatus::Active,
            token_budget: Some(1000),
            tokens_used: 250,
            time_used_seconds: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
            continuation_suppressed: false,
        };
        let block = active_goal_context_block(&goal).unwrap();
        assert!(block.contains("[active_goal]"));
        assert!(block.contains("ship the feature"));
        assert!(block.contains("250 used / 1000"));
        assert!(block.contains("goal_complete"));
    }

    #[test]
    fn budget_limited_block_steers_to_summarise() {
        let goal = ThreadGoal {
            thread_id: "t".into(),
            goal_id: "g".into(),
            objective: "obj".into(),
            status: ThreadGoalStatus::BudgetLimited,
            token_budget: Some(100),
            tokens_used: 100,
            time_used_seconds: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
            continuation_suppressed: false,
        };
        let block = active_goal_context_block(&goal).unwrap();
        assert!(block.contains("reached its token budget"));
    }

    #[test]
    fn paused_and_complete_render_no_block() {
        let mut goal = ThreadGoal {
            thread_id: "t".into(),
            goal_id: "g".into(),
            objective: "obj".into(),
            status: ThreadGoalStatus::Paused,
            token_budget: None,
            tokens_used: 0,
            time_used_seconds: 0,
            created_at_ms: 0,
            updated_at_ms: 0,
            continuation_suppressed: false,
        };
        assert!(active_goal_context_block(&goal).is_none());
        goal.status = ThreadGoalStatus::Complete;
        assert!(active_goal_context_block(&goal).is_none());
    }

    #[tokio::test]
    async fn account_turn_charges_active_goal_and_trips_budget() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        crate::openhuman::inference::provider::thread_context::with_thread_id("t-acct", async {
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
        crate::openhuman::inference::provider::thread_context::with_thread_id("t-paused", async {
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
