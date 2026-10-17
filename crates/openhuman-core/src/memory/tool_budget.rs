//! OpenHuman's latency and call budget for optional memory in a chat turn.
//! Keeps 128 run records; outstanding reservations cannot be evicted. Idle
//! records may age out under churn, so aggregate limits apply while tracked.

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;
use tinytools::ToolResult;

const CALL_TIMEOUT: Duration = Duration::from_secs(15);
const RUN_TIME_BUDGET: Duration = Duration::from_secs(30);
const MAX_CALLS: usize = 8;
const MAX_TRACKED_RUNS: usize = 128;
const CONTINUE: &str = "Continue answering the user's request without memory. Do not retry memory in this run. A timed-out write may already have been accepted; do not claim it failed or repeat it.";
/// Host-only metadata key on a call the budget refused without running it.
const REFUSAL_METADATA_KEY: &str = "openhuman.memory_budget";

/// A refusal: the budget's own verdict, not the tool failing.
fn refusal(text: String) -> ToolResult {
    ToolResult::error(text).with_metadata(serde_json::json!({ REFUSAL_METADATA_KEY: "refused" }))
}

/// Whether the budget refused `result` without running the action. Failure
/// ladders skip these: a parallel batch past the cap must not halt the run.
pub fn is_budget_refusal(result: &ToolResult) -> bool {
    result
        .metadata
        .as_ref()
        .is_some_and(|metadata| metadata.get(REFUSAL_METADATA_KEY).is_some())
}

#[derive(Default)]
pub(super) struct ToolBudget {
    runs: Mutex<Runs>,
}

#[derive(Default)]
struct Runs {
    states: HashMap<String, RunBudget>,
    order: VecDeque<String>,
}

struct RunBudget {
    calls: usize,
    remaining: Duration,
    disabled: bool,
    in_flight: usize,
}

impl Default for RunBudget {
    fn default() -> Self {
        Self {
            calls: 0,
            remaining: RUN_TIME_BUDGET,
            disabled: false,
            in_flight: 0,
        }
    }
}

/// Pins a run until its action returns or the caller drops the future. Keeping
/// settlement in Drop also refunds unused time on cooperative cancellation.
struct Reservation<'a> {
    budget: &'a ToolBudget,
    run_id: Option<String>,
    allowance: Duration,
    started: tokio::time::Instant,
    timed_out: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        let Some(run_id) = self.run_id.as_deref() else {
            return;
        };
        let mut runs = self
            .budget
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(state) = runs.states.get_mut(run_id) {
            state.remaining += self.allowance.saturating_sub(self.started.elapsed());
            state.disabled |= self.timed_out;
            state.in_flight -= 1;
        }
    }
}

impl ToolBudget {
    // Reserve time before polling the operation, so parallel reads share the
    // same budget. Return unused time on completion; never hold a lock across
    // engine I/O. State is scoped to a harness run, not to the conversation.
    fn reserve(&self, run_id: Option<&str>) -> Result<Duration, ToolResult> {
        let Some(run_id) = run_id else {
            return Ok(CALL_TIMEOUT);
        };
        let mut runs = self
            .runs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !runs.states.contains_key(run_id) {
            if runs.order.len() == MAX_TRACKED_RUNS {
                let Some(index) = runs
                    .order
                    .iter()
                    .position(|id| runs.states[id].in_flight == 0)
                else {
                    tracing::debug!(run_id, cutoff = "tracking_full", "[memory:budget] cutoff");
                    return Err(refusal(format!(
                        "Memory is busy with other runs. {CONTINUE}"
                    )));
                };
                let oldest = runs.order.remove(index).expect("position is in the queue");
                runs.states.remove(&oldest);
            }
            runs.order.push_back(run_id.to_string());
        }
        let state = runs.states.entry(run_id.to_string()).or_default();
        if state.disabled || state.calls >= MAX_CALLS || state.remaining.is_zero() {
            tracing::debug!(
                run_id,
                cutoff = "exhausted",
                calls = state.calls,
                disabled = state.disabled,
                remaining_ms = state.remaining.as_millis(),
                "[memory:budget] cutoff"
            );
            return Err(refusal(format!(
                "Memory budget exhausted for this run. {CONTINUE}"
            )));
        }
        state.calls += 1;
        state.in_flight += 1;
        let allowance = state.remaining.min(CALL_TIMEOUT);
        state.remaining -= allowance;
        Ok(allowance)
    }

    /// Bounds optional memory work. A timeout opens the circuit for this run;
    /// the next run gets a fresh budget. Unscoped callers still get a deadline.
    pub(super) async fn run(
        &self,
        run_id: Option<String>,
        action: impl Future<Output = ToolResult>,
    ) -> ToolResult {
        let allowance = match self.reserve(run_id.as_deref()) {
            Ok(allowance) => allowance,
            Err(result) => return result,
        };
        let mut reservation = Reservation {
            budget: self,
            run_id,
            allowance,
            started: tokio::time::Instant::now(),
            timed_out: false,
        };
        let result = tokio::time::timeout(allowance, action).await;
        reservation.timed_out = result.is_err();
        if reservation.timed_out {
            tracing::debug!(
                run_id = reservation.run_id.as_deref().unwrap_or("unscoped"),
                cutoff = "timeout",
                allowance_ms = allowance.as_millis(),
                "[memory:budget] cutoff"
            );
        }
        drop(reservation);
        result.unwrap_or_else(|_| ToolResult::error(format!(
            "Memory did not finish within its latency budget and is unavailable for this run. {CONTINUE}"
        )))
    }
}

#[cfg(test)]
#[path = "tool_budget_tests.rs"]
mod tests;
