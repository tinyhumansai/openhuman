//! Turn-scoped guard that decides whether a **new sub-agent dispatch can still
//! succeed** before it is attempted.
//!
//! # The failure this exists to prevent
//!
//! A turn that reaches its model-call cap asks the harness to stop gracefully
//! ([`CapPauser`](crate::openhuman::agent::tinyagents::observability), which
//! sends `SteeringCommand::Pause`) so the caller can summarise a resumable
//! checkpoint instead of erroring. That command is **advisory**: it is honoured
//! at the harness loop boundary, and nothing consulted it before dispatching a
//! new sub-agent. A dispatch issued in the same instant as the pause request
//! then runs as a tool call wrapped by the run's *remaining* wall-clock budget
//! (`run_policy_for`'s `max_wall_clock_ms`). When that remainder is smaller
//! than the child needs, the harness raises `TinyAgentsError::Timeout` for the
//! whole run — and every result the turn had accumulated is discarded rather
//! than checkpointed. The mechanism designed to degrade gracefully became the
//! cause of the hard failure (issue #5804).
//!
//! # What this guard does
//!
//! It records two facts a turn already knows but never wrote down, and turns
//! them into a decision at the one chokepoint every synchronous delegation
//! passes through (`subagent_runner::ops::runner::run_subagent`):
//!
//! 1. **Has a graceful pause been requested?** Once it has, further fan-out
//!    cannot help: the loop is going to stop at its next boundary regardless,
//!    so a child dispatched now can only burn budget the checkpoint needs.
//! 2. **Is there still enough wall-clock left for a dispatch to finish?**
//!    Judged against what *this turn's own* sub-agents have actually taken —
//!    never a configured constant.
//!
//! # Scope semantics
//!
//! Installed as a `tokio::task_local` around the parent's turn future, next to
//! [`turn_subagent_usage`](super::turn_subagent_usage) and with deliberately
//! identical visibility rules. The line is **same task or not**, which is not
//! the same line as serial or parallel:
//!
//! * **Serial delegation** (`spawn_subagent` → `run_subagent`) runs inline on
//!   the turn's task and is covered.
//! * **Parallel fan-out is also covered.** `spawn_parallel_agents` drives its
//!   workers through `tinyagents_graph::parallel::map_reduce`, which bounds
//!   concurrency with `futures`' `buffer_unordered`
//!   (`vendor/tinyagents/src/graph/parallel/mod.rs:86`) rather than
//!   `tokio::spawn`. `buffer_unordered` polls every worker on the **caller's**
//!   task, so each one inherits this task-local and passes the same gate. Its
//!   serial fallback for shared-workspace writes is covered for the obvious
//!   reason. Both call `run_subagent`
//!   (`openhuman:src/openhuman/agent/orchestration/spawn_parallel_graph.rs:1386`).
//! * **Detached background sub-agents** (`spawn_async_subagent`) run on tasks
//!   that do not inherit the task-local, deliberately — their spend already
//!   completes after the parent's `chat_done` and is accounted globally, the
//!   same carve-out `turn_subagent_usage` makes.
//!
//! [`current`] returns `None` outside any scope (CLI, direct invocation,
//! tests), and **every read degrades to "allow"** — this guard can only ever
//! refuse a dispatch it has positive evidence against, so a missing scope
//! restores exactly the previous behaviour.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Per-turn dispatch state, shared between the turn scope, the harness's
/// cap-pause listener, and the sub-agent runner.
#[derive(Debug)]
pub struct TurnDispatchState {
    /// Set once the harness has been asked to pause gracefully at the
    /// model-call cap. Written by the cap-pause listener through a clone of
    /// this `Arc` rather than through the task-local, so correctness does not
    /// depend on which task the crate dispatches listeners from.
    pause_requested: AtomicBool,
    /// Model calls completed when the pause was requested, and the cap that
    /// triggered it. Carried only so the refusal can explain itself.
    pause_completed_calls: AtomicU64,
    pause_cap: AtomicU64,
    /// When this turn's wall-clock budget started, and how long it is. `None`
    /// budget means the ceiling is disabled (`OPENHUMAN_AGENT_TURN_TIMEOUT_SECS=0`),
    /// in which case no dispatch can ever be refused for want of time.
    started: Instant,
    budget: Option<Duration>,
    /// Longest sub-agent this turn has actually completed, in milliseconds.
    /// `0` means "no sample yet". A running maximum rather than a stored list:
    /// the decision only ever needs the extreme, and a `u64` keeps the whole
    /// state lock-free.
    observed_max_ms: AtomicU64,
    /// How many completed sub-agents fed `observed_max_ms`, for the log line
    /// and the refusal text.
    observed_samples: AtomicU64,
}

impl TurnDispatchState {
    fn new(budget: Option<Duration>) -> Self {
        Self {
            pause_requested: AtomicBool::new(false),
            pause_completed_calls: AtomicU64::new(0),
            pause_cap: AtomicU64::new(0),
            started: Instant::now(),
            budget,
            observed_max_ms: AtomicU64::new(0),
            observed_samples: AtomicU64::new(0),
        }
    }

    /// Record that a graceful pause has been requested at the model-call cap.
    ///
    /// Called from the harness's event listener. The crate dispatches listeners
    /// synchronously on the emitting task, in insertion order
    /// (`vendor/tinyagents/src/harness/events/mod.rs:163-195`), so this write
    /// happens-before any later tool dispatch in the same run — which is what
    /// makes the gate deterministic rather than a second race against the
    /// advisory `SteeringCommand::Pause`.
    pub fn record_pause_requested(&self, completed_model_calls: u64, cap: u64) {
        self.pause_completed_calls
            .store(completed_model_calls, Ordering::SeqCst);
        self.pause_cap.store(cap, Ordering::SeqCst);
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    /// Fold one completed sub-agent's wall-clock duration into the running
    /// maximum.
    fn record_subagent_elapsed(&self, elapsed: Duration) {
        let ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        self.observed_samples.fetch_add(1, Ordering::SeqCst);
        self.observed_max_ms.fetch_max(ms, Ordering::SeqCst);
    }

    /// Wall-clock still available to this turn, or `None` when no ceiling is
    /// configured.
    fn remaining(&self) -> Option<Duration> {
        self.budget
            .map(|b| b.saturating_sub(self.started.elapsed()))
    }

    /// Longest completed sub-agent so far, or `None` when none has completed.
    fn observed_max(&self) -> Option<Duration> {
        match self.observed_max_ms.load(Ordering::SeqCst) {
            0 => None,
            ms => Some(Duration::from_millis(ms)),
        }
    }

    fn snapshot(&self) -> DispatchInputs {
        DispatchInputs {
            pause_requested: self.pause_requested.load(Ordering::SeqCst),
            pause_completed_calls: self.pause_completed_calls.load(Ordering::SeqCst),
            pause_cap: self.pause_cap.load(Ordering::SeqCst),
            remaining: self.remaining(),
            observed_max: self.observed_max(),
            observed_samples: self.observed_samples.load(Ordering::SeqCst),
        }
    }
}

/// Everything [`decide`] is allowed to look at. Extracted as a plain value so
/// the policy is unit-testable without a runtime, a clock, or a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchInputs {
    pub pause_requested: bool,
    pub pause_completed_calls: u64,
    pub pause_cap: u64,
    pub remaining: Option<Duration>,
    pub observed_max: Option<Duration>,
    pub observed_samples: u64,
}

/// Outcome of the pre-dispatch check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDecision {
    /// Nothing known contradicts this dispatch. The default in every uncertain
    /// case.
    Allow,
    /// A graceful pause has already been requested for this turn.
    RefusePaused {
        completed_model_calls: u64,
        cap: u64,
    },
    /// Less wall-clock remains than this turn's slowest completed sub-agent
    /// took.
    RefuseBudget {
        remaining_ms: u64,
        observed_max_ms: u64,
        observed_samples: u64,
    },
}

/// Decide whether a sub-agent dispatch should be attempted.
///
/// Two independent refusals, in order of certainty:
///
/// 1. **A pause was requested.** This is a fact about intent, not an estimate:
///    the loop will stop at its next boundary whatever we do, so a child
///    started now cannot contribute to the answer and can only consume the
///    budget the checkpoint summary needs.
///
/// 2. **The remaining budget is smaller than the longest sub-agent this turn
///    has actually completed.** The predictor is the turn's own measured
///    maximum — never a configured constant, a model name, a task shape, or
///    the number of children so far — so it is equally meaningful for a turn
///    with three fast children and one with three hundred slow ones, and it
///    means nothing at all until this turn has produced its first sample.
///
/// The maximum is used rather than a mean or a percentile because the two
/// errors are not symmetric. Refusing a dispatch that would have fit costs one
/// delegation and still returns every result gathered so far; allowing one that
/// does not fit costs **the entire turn**. Under that asymmetry the
/// conservative estimator is the correct one, and it is also the only one that
/// needs no tunable.
///
/// Both refusals are strictly evidence-based: with no pause requested, no
/// configured ceiling, or no completed sub-agent to learn from, the answer is
/// [`DispatchDecision::Allow`].
pub fn decide(inputs: DispatchInputs) -> DispatchDecision {
    if inputs.pause_requested {
        return DispatchDecision::RefusePaused {
            completed_model_calls: inputs.pause_completed_calls,
            cap: inputs.pause_cap,
        };
    }

    // `remaining` is `None` when the wall-clock ceiling is disabled, and
    // `observed_max` is `None` until a sub-agent has finished. Either one
    // absent means there is nothing to compare, so nothing to refuse.
    let (Some(remaining), Some(observed_max)) = (inputs.remaining, inputs.observed_max) else {
        return DispatchDecision::Allow;
    };

    if remaining < observed_max {
        return DispatchDecision::RefuseBudget {
            remaining_ms: remaining.as_millis().min(u128::from(u64::MAX)) as u64,
            observed_max_ms: observed_max.as_millis().min(u128::from(u64::MAX)) as u64,
            observed_samples: inputs.observed_samples,
        };
    }

    DispatchDecision::Allow
}

tokio::task_local! {
    /// Dispatch state for the turn currently executing on this task. Absent
    /// outside a turn scope.
    static TURN_DISPATCH_STATE: Arc<TurnDispatchState>;
}

/// The dispatch state for the current turn, or `None` outside any scope.
pub fn current() -> Option<Arc<TurnDispatchState>> {
    TURN_DISPATCH_STATE.try_with(|s| s.clone()).ok()
}

/// Ask the guard whether a sub-agent may be dispatched now.
///
/// Returns [`DispatchDecision::Allow`] when no guard is installed, so every
/// path that does not run under a turn scope behaves exactly as before.
pub fn check() -> DispatchDecision {
    match current() {
        Some(state) => decide(state.snapshot()),
        None => DispatchDecision::Allow,
    }
}

/// Fold a finished sub-agent's wall-clock duration into the current turn's
/// observed maximum. No-op outside a turn scope.
pub fn record_subagent_elapsed(elapsed: Duration) {
    if let Some(state) = current() {
        state.record_subagent_elapsed(elapsed);
    }
}

/// Run `future` with a fresh dispatch guard installed for `budget`.
///
/// `budget` is the turn's wall-clock ceiling — the same value the harness
/// policy receives as `max_wall_clock_ms`. Measuring from here rather than from
/// the harness run means the guard's idea of "elapsed" starts marginally
/// *earlier* than the harness's, so its remaining-budget figure is
/// conservative: it can under-estimate the time left, never over-estimate it.
pub async fn with_dispatch_guard<F, R>(budget: Option<Duration>, future: F) -> R
where
    F: std::future::Future<Output = R>,
{
    let state = Arc::new(TurnDispatchState::new(budget));
    // `scope` takes the inner future by value; the turn generator is hundreds
    // of KiB in a debug build, so pin it to the heap first and move only a
    // pointer through this frame. See the same note (and the stack-overflow
    // measurements behind it) in `turn_subagent_usage::with_turn_collector`.
    TURN_DISPATCH_STATE.scope(state, Box::pin(future)).await
}

#[cfg(test)]
#[path = "turn_dispatch_guard_tests.rs"]
mod tests;
