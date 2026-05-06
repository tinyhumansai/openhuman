//! Mid-turn stop hooks — policy-driven halt of an in-flight agent
//! turn.
//!
//! Distinct from [`super::harness::interrupt::InterruptFence`], which
//! handles user-driven cancellation (Ctrl+C / `/stop`). Stop hooks are
//! the policy lever: budget caps, rate limits, custom kill switches.
//! They run between iterations of the tool-call loop so a runaway
//! turn can be cut short before the next provider call rather than
//! after the fact.
//!
//! ## Wiring
//!
//! Hooks ride on a task-local rather than a parameter on
//! [`crate::openhuman::agent::harness::tool_loop::run_tool_call_loop`]
//! — that signature already takes 16 args and the function is invoked
//! from a dozen+ call sites. The task-local mirrors how
//! [`super::harness::fork_context::PARENT_CONTEXT`] and
//! [`super::harness::sandbox_context::CURRENT_AGENT_SANDBOX_MODE`] are
//! threaded.
//!
//! Callers register hooks via [`with_stop_hooks`] around their
//! [`Agent::run_single`] / `run_interactive` invocation; the loop
//! reads them via [`current_stop_hooks`] and fires them at the top of
//! each iteration. A hook returning [`StopDecision::Stop`] aborts the
//! loop with a [`StoppedByHookError`]-shaped `anyhow` error so the
//! caller can surface the reason to the user.
//!
//! ## Built-in hooks
//!
//! - [`BudgetStopHook`] — caps cumulative turn cost in USD using the
//!   [`super::cost::TurnCost`] accumulator.
//! - [`MaxIterationsStopHook`] — caps iteration count from outside the
//!   `max_tool_iterations` config (useful for ad-hoc per-call limits
//!   without mutating the agent's persistent config).

use crate::openhuman::agent::cost::TurnCost;
use async_trait::async_trait;
use std::sync::Arc;

/// A policy hook fired between iterations of the tool-call loop.
#[async_trait]
pub trait StopHook: Send + Sync {
    /// Stable name for tracing / error messages (e.g. `"budget"`).
    fn name(&self) -> &str;

    /// Inspect the current turn state and decide whether to continue.
    async fn check(&self, ctx: &TurnState<'_>) -> StopDecision;
}

/// Outcome of a single hook check.
#[derive(Debug, Clone)]
pub enum StopDecision {
    /// Keep the loop running.
    Continue,
    /// Stop the loop. `reason` is propagated to the caller.
    Stop { reason: String },
}

/// Snapshot of the turn at the moment a hook fires. References are
/// borrowed from the loop's locals so hooks pay no allocation cost on
/// the hot path; clone fields out if you need to keep them.
pub struct TurnState<'a> {
    /// 1-based iteration index that's about to start.
    pub iteration: u32,
    /// Configured iteration cap for this turn.
    pub max_iterations: u32,
    /// Cumulative cost / token tally so far.
    pub cost: &'a TurnCost,
    /// Model name passed to this turn's provider calls.
    pub model: &'a str,
}

tokio::task_local! {
    /// Active stop hooks. `None` (the task-local-not-set state) is
    /// treated as "no hooks" — see [`current_stop_hooks`].
    pub static CURRENT_STOP_HOOKS: Vec<Arc<dyn StopHook>>;
}

/// Returns a clone of the currently-installed hook list, or an empty
/// vec when no scope has been entered.
pub fn current_stop_hooks() -> Vec<Arc<dyn StopHook>> {
    CURRENT_STOP_HOOKS
        .try_with(|hooks| hooks.clone())
        .unwrap_or_default()
}

/// Run `future` with `hooks` installed as the active stop-hook list.
pub async fn with_stop_hooks<F, R>(hooks: Vec<Arc<dyn StopHook>>, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    CURRENT_STOP_HOOKS.scope(hooks, future).await
}

// ─────────────────────────────────────────────────────────────────────────────
// Built-in hooks
// ─────────────────────────────────────────────────────────────────────────────

/// Stop the turn once cumulative cost reaches `max_usd`.
///
/// Uses [`TurnCost::total_usd`] which prefers the backend's
/// `charged_amount_usd` and falls back to a tier-keyed estimate.
#[derive(Debug, Clone, Copy)]
pub struct BudgetStopHook {
    pub max_usd: f64,
}

impl BudgetStopHook {
    pub fn new(max_usd: f64) -> Self {
        Self { max_usd }
    }
}

#[async_trait]
impl StopHook for BudgetStopHook {
    fn name(&self) -> &str {
        "budget"
    }

    async fn check(&self, ctx: &TurnState<'_>) -> StopDecision {
        // Fail closed on a malformed cap: NaN, non-finite, or
        // non-positive `max_usd` should *stop* rather than silently
        // disable the guard (NaN comparisons always return false, so
        // `spent >= NaN` would otherwise let the loop run forever).
        if !self.max_usd.is_finite() || self.max_usd <= 0.0 {
            return StopDecision::Stop {
                reason: format!("invalid budget cap configured: max_usd={}", self.max_usd),
            };
        }
        let spent = ctx.cost.total_usd();
        if spent >= self.max_usd {
            StopDecision::Stop {
                reason: format!(
                    "turn cost ${spent:.4} reached cap ${cap:.4}",
                    cap = self.max_usd
                ),
            }
        } else {
            StopDecision::Continue
        }
    }
}

/// Stop the turn at a hard iteration ceiling.
///
/// Sibling of `max_tool_iterations` on `AgentConfig`; this hook is
/// useful when callers want to lower the limit for one specific turn
/// without mutating the agent's persistent config.
#[derive(Debug, Clone, Copy)]
pub struct MaxIterationsStopHook {
    pub cap: u32,
}

impl MaxIterationsStopHook {
    pub fn new(cap: u32) -> Self {
        Self { cap }
    }
}

#[async_trait]
impl StopHook for MaxIterationsStopHook {
    fn name(&self) -> &str {
        "max_iterations"
    }

    async fn check(&self, ctx: &TurnState<'_>) -> StopDecision {
        if ctx.iteration > self.cap {
            StopDecision::Stop {
                reason: format!(
                    "turn reached iteration cap {} (about to start iteration {})",
                    self.cap, ctx.iteration
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
    use crate::openhuman::providers::UsageInfo;

    fn cost_with_usd(usd: f64) -> TurnCost {
        let mut tc = TurnCost::new();
        tc.add_call(
            "agentic-v1",
            &UsageInfo {
                charged_amount_usd: usd,
                ..Default::default()
            },
        );
        tc
    }

    #[tokio::test]
    async fn budget_hook_continues_under_cap() {
        let cost = cost_with_usd(0.10);
        let hook = BudgetStopHook::new(1.00);
        let ctx = TurnState {
            iteration: 1,
            max_iterations: 10,
            cost: &cost,
            model: "agentic-v1",
        };
        assert!(matches!(hook.check(&ctx).await, StopDecision::Continue));
    }

    #[tokio::test]
    async fn budget_hook_stops_at_cap() {
        let cost = cost_with_usd(1.50);
        let hook = BudgetStopHook::new(1.00);
        let ctx = TurnState {
            iteration: 2,
            max_iterations: 10,
            cost: &cost,
            model: "agentic-v1",
        };
        match hook.check(&ctx).await {
            StopDecision::Stop { reason } => {
                assert!(reason.contains("$1.5000"));
                assert!(reason.contains("$1.0000"));
            }
            other => panic!("expected Stop, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn budget_hook_fails_closed_on_nan_cap() {
        // NaN comparisons always return false, so without the guard
        // `spent >= NaN` would silently disable the cap forever.
        let cost = cost_with_usd(1.0);
        let hook = BudgetStopHook::new(f64::NAN);
        let ctx = TurnState {
            iteration: 1,
            max_iterations: 10,
            cost: &cost,
            model: "agentic-v1",
        };
        match hook.check(&ctx).await {
            StopDecision::Stop { reason } => assert!(reason.contains("invalid budget cap")),
            other => panic!("expected Stop on NaN cap, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn budget_hook_fails_closed_on_non_positive_cap() {
        let cost = TurnCost::new();
        let ctx = TurnState {
            iteration: 1,
            max_iterations: 10,
            cost: &cost,
            model: "agentic-v1",
        };
        for bad in [0.0, -1.0, f64::NEG_INFINITY, f64::INFINITY] {
            let hook = BudgetStopHook::new(bad);
            assert!(
                matches!(hook.check(&ctx).await, StopDecision::Stop { .. }),
                "cap {bad} should stop"
            );
        }
    }

    #[tokio::test]
    async fn max_iterations_hook_stops_when_exceeded() {
        let cost = TurnCost::new();
        let hook = MaxIterationsStopHook::new(3);
        let ctx = TurnState {
            iteration: 4,
            max_iterations: 10,
            cost: &cost,
            model: "agentic-v1",
        };
        assert!(matches!(hook.check(&ctx).await, StopDecision::Stop { .. }));
    }

    #[tokio::test]
    async fn current_stop_hooks_returns_empty_outside_scope() {
        assert!(current_stop_hooks().is_empty());
    }

    #[tokio::test]
    async fn with_stop_hooks_installs_visible_within_scope() {
        let hooks: Vec<Arc<dyn StopHook>> = vec![Arc::new(BudgetStopHook::new(0.5))];
        with_stop_hooks(hooks, async {
            let visible = current_stop_hooks();
            assert_eq!(visible.len(), 1);
            assert_eq!(visible[0].name(), "budget");
        })
        .await;
        assert!(current_stop_hooks().is_empty());
    }
}
