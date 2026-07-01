//! Mid-turn stop hooks as a `tinyagents` middleware (issue #4249).
//!
//! openhuman's policy-driven [`StopHook`]s (budget cap, thread-goal budget,
//! ad-hoc iteration ceiling) used to fire between iterations of the legacy
//! tool-call loop. That loop is gone — every turn now runs on the `tinyagents`
//! harness — so the firing point moves into a [`Middleware`]:
//!
//! - [`Middleware::after_model`] accumulates each completed model call's usage
//!   into a per-turn [`TurnCost`] (the same accounting the hooks read), then
//!   evaluates every installed hook with a [`TurnState`] snapshot.
//! - On the first [`StopDecision::Stop`], it sends [`SteeringCommand::Pause`] on
//!   the run's steering handle. The agent loop drains steering at the **top** of
//!   the next iteration (before the next model call) and `Pause` short-circuits
//!   it cleanly — so the run stops before spending another call, returning the
//!   partial transcript. This mirrors the [`CapPauser`](super::CapPauser)
//!   model-call-cap mechanism.
//!
//! The hook list is captured by the caller via
//! [`current_stop_hooks`](crate::openhuman::agent::stop_hooks::current_stop_hooks)
//! while the `CURRENT_STOP_HOOKS` task-local is in scope and handed to
//! [`StopHookMiddleware::new`]; the middleware is only registered when the list
//! is non-empty.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use tinyagents::harness::context::RunContext;
use tinyagents::harness::middleware::Middleware;
use tinyagents::harness::model::ModelResponse;
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};

use crate::openhuman::agent::cost::TurnCost;
use crate::openhuman::agent::stop_hooks::{StopDecision, StopHook, TurnState};
use crate::openhuman::inference::provider::UsageInfo;

/// Fires openhuman [`StopHook`]s after each model call and pauses the run when
/// any hook votes to stop.
pub struct StopHookMiddleware {
    /// Steering handle the run was built with; `Pause` is sent here on stop.
    handle: SteeringHandle,
    /// Model name reported to hooks (and used for cost estimation).
    model: String,
    /// Configured model-call ceiling for this turn (the hooks' `max_iterations`).
    max_iterations: u32,
    /// 0-based count of completed model calls; incremented in `after_model`.
    iteration: AtomicU32,
    /// Per-turn usage tally the hooks read (`TurnState::cost`).
    cost: Mutex<TurnCost>,
    /// Installed hooks, snapshotted from the task-local at construction.
    hooks: Vec<Arc<dyn StopHook>>,
    /// Latches once a hook has voted to stop, so we send `Pause` exactly once.
    stopped: AtomicBool,
}

impl StopHookMiddleware {
    /// Build a middleware firing `hooks`, pausing `handle` on the first stop.
    pub fn new(
        handle: SteeringHandle,
        model: impl Into<String>,
        max_iterations: usize,
        hooks: Vec<Arc<dyn StopHook>>,
    ) -> Self {
        Self {
            handle,
            model: model.into(),
            max_iterations: max_iterations.min(u32::MAX as usize) as u32,
            iteration: AtomicU32::new(0),
            cost: Mutex::new(TurnCost::new()),
            hooks,
            stopped: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl<State, Ctx> Middleware<State, Ctx> for StopHookMiddleware
where
    State: Send + Sync,
    Ctx: Send + Sync,
{
    fn name(&self) -> &str {
        "openhuman.stop_hooks"
    }

    async fn after_model(
        &self,
        _ctx: &mut RunContext<Ctx>,
        _state: &State,
        response: &mut ModelResponse,
    ) -> tinyagents::Result<()> {
        // Already paused: nothing more to do (Pause is latched once).
        if self.stopped.load(Ordering::SeqCst) {
            return Ok(());
        }

        // Fold this call's usage into the running turn cost (the same tally the
        // budget/goal hooks read). Clone it back out so we never hold the mutex
        // guard across the async hook checks below.
        let iteration = self.iteration.fetch_add(1, Ordering::SeqCst) + 1;
        let cost_snapshot = {
            let mut cost = self.cost.lock().expect("stop-hook cost mutex poisoned");
            if let Some(usage) = &response.usage {
                cost.add_call(
                    &self.model,
                    &UsageInfo {
                        input_tokens: usage.input_tokens,
                        output_tokens: usage.output_tokens,
                        context_window: 0,
                        cached_input_tokens: usage.cache_read_tokens,
                        charged_amount_usd: 0.0,
                    },
                );
            }
            cost.clone()
        };

        let turn_state = TurnState {
            iteration,
            max_iterations: self.max_iterations,
            cost: &cost_snapshot,
            model: &self.model,
        };

        for hook in &self.hooks {
            if let StopDecision::Stop { reason } = hook.check(&turn_state).await {
                // Latch first so a concurrent (streaming) after_model can't
                // double-pause.
                if self.stopped.swap(true, Ordering::SeqCst) {
                    return Ok(());
                }
                tracing::warn!(
                    target: "stop_hooks",
                    hook = hook.name(),
                    iteration,
                    model = %self.model,
                    "[stop_hooks] hook voted to stop the turn — pausing run: {reason}"
                );
                // Graceful stop: the loop drains steering at the top of the next
                // iteration and `Pause` short-circuits it before the next model
                // call. The partial transcript is returned to the caller.
                self.handle.send(SteeringCommand::Pause);
                return Ok(());
            }
        }

        Ok(())
    }
}
