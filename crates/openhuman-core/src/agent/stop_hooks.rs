//! Mid-turn stop hooks — policy-driven halt of an in-flight agent
//! turn.
//!
//! Stop hooks are the policy lever: budget caps, rate limits, custom kill
//! switches. They run between iterations of the agent loop so a runaway turn can
//! be cut short before the next provider call rather than after the fact.
//! (User-driven cancellation — Ctrl+C / `/stop` — is handled separately by the
//! `tinyagents` steering/cancellation channel.)
//!
//! The `tinyagents` adapter installs a `StopHookMiddleware`
//! ([`crate::agent::tinyagents::stop_hooks`]) that fires each hook after
//! every model call; a hook returning [`StopDecision::Stop`] pauses the run
//! gracefully (via the steering handle) before the next provider call. Hooks
//! arrive on the turn's explicit run context
//! (`OpenHumanRunContext::stop_hooks`). The
//! goal-budget hook ([`crate::agent::goals::runtime::GoalBudgetStopHook`]) is
//! the live implementation.
//!
//! The per-turn tool budget ([`tool_call_limit`]) also lives here; a host
//! narrows it through `HostTurnEnv::tool_call_limit`.

use async_trait::async_trait;
use tinyagents_harness::cost::TurnCost;

use tinyagents_harness::middleware::{StopDecision, StopHook, TurnState};

/// Stop a turn when its cumulative cost reaches the configured USD cap.
#[derive(Debug, Clone, Copy)]
pub struct BudgetStopHook {
    /// Cumulative USD charge at which the turn pauses.
    pub max_usd: f64,
}

impl BudgetStopHook {
    /// Construct a hook that pauses after a turn reaches `max_usd`.
    pub fn new(max_usd: f64) -> Self {
        Self { max_usd }
    }
}

#[async_trait]
impl StopHook<TurnCost> for BudgetStopHook {
    fn name(&self) -> &str {
        "budget"
    }

    async fn check(&self, ctx: &TurnState<'_, TurnCost>) -> StopDecision {
        if !self.max_usd.is_finite() || self.max_usd <= 0.0 {
            return StopDecision::Stop {
                reason: format!("invalid budget cap configured: max_usd={}", self.max_usd),
            };
        }
        // A cap can only bite on spend it can see: calls of unknown cost add
        // nothing here rather than a made-up rate.
        let spent = ctx.cost.cost.known_usd;
        if spent >= self.max_usd {
            StopDecision::Stop {
                reason: format!("turn cost ${spent:.4} reached cap ${:.4}", self.max_usd),
            }
        } else {
            StopDecision::Continue
        }
    }
}

/// The real tool invocation budget for one turn: the default derived from the
/// iteration cap, narrowed by the host's explicit `cap`
/// (`HostTurnEnv::tool_call_limit`) when it set one. A host can only narrow it.
pub(crate) fn tool_call_limit(max_iterations: usize, cap: Option<usize>) -> usize {
    let default = max_iterations.saturating_mul(8).max(8);
    cap.map_or(default, |cap| default.min(cap))
}

#[cfg(test)]
#[path = "stop_hooks_tests.rs"]
mod tests;
