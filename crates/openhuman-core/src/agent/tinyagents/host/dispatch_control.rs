//! Host-owned pre-dispatch policy and per-turn evidence used to decide whether a child may start.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Immutable inputs to the host's pre-dispatch policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DispatchInputs {
    pub pause_requested: bool,
    pub pause_completed_calls: u64,
    pub pause_cap: u64,
    pub remaining: Option<Duration>,
    pub observed_max: Option<Duration>,
    pub observed_samples: u64,
}

/// Outcome of the host's pre-dispatch policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchDecision {
    Allow,
    RefusePaused {
        completed_model_calls: u64,
        cap: u64,
    },
    RefuseBudget {
        remaining_ms: u64,
        observed_max_ms: u64,
        observed_samples: u64,
    },
}

/// Decide whether a child can be started using only evidence recorded on this
/// turn. This stays host policy: TinyAgents owns the loop, not OpenHuman's
/// pause and wall-clock refusal rules.
pub fn decide_dispatch(inputs: DispatchInputs) -> DispatchDecision {
    if inputs.pause_requested {
        return DispatchDecision::RefusePaused {
            completed_model_calls: inputs.pause_completed_calls,
            cap: inputs.pause_cap,
        };
    }
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

/// Shared root-turn state used by all synchronous descendants and siblings to
/// decide whether starting another delegate is still safe.
#[derive(Debug)]
pub struct TurnDispatchState {
    pause_requested: AtomicBool,
    pause_completed_calls: AtomicU64,
    pause_cap: AtomicU64,
    started: Instant,
    budget: Option<Duration>,
    observed_max_ms: AtomicU64,
    observed_samples: AtomicU64,
}

impl TurnDispatchState {
    pub fn new(budget: Option<Duration>) -> Self {
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

    pub fn record_pause_requested(&self, completed_model_calls: u64, cap: u64) {
        self.pause_completed_calls
            .store(completed_model_calls, Ordering::SeqCst);
        self.pause_cap.store(cap, Ordering::SeqCst);
        self.pause_requested.store(true, Ordering::SeqCst);
    }

    pub fn record_subagent_elapsed(&self, elapsed: Duration) {
        let ms = elapsed.as_millis().min(u128::from(u64::MAX)) as u64;
        self.observed_samples.fetch_add(1, Ordering::SeqCst);
        self.observed_max_ms.fetch_max(ms, Ordering::SeqCst);
    }

    pub fn snapshot(&self) -> DispatchInputs {
        let observed_max = match self.observed_max_ms.load(Ordering::SeqCst) {
            0 => None,
            ms => Some(Duration::from_millis(ms)),
        };
        DispatchInputs {
            pause_requested: self.pause_requested.load(Ordering::SeqCst),
            pause_completed_calls: self.pause_completed_calls.load(Ordering::SeqCst),
            pause_cap: self.pause_cap.load(Ordering::SeqCst),
            remaining: self
                .budget
                .map(|budget| budget.saturating_sub(self.started.elapsed())),
            observed_max,
            observed_samples: self.observed_samples.load(Ordering::SeqCst),
        }
    }

    pub fn check(&self) -> DispatchDecision {
        decide_dispatch(self.snapshot())
    }
}

#[cfg(test)]
#[path = "dispatch_control_tests.rs"]
mod tests;
