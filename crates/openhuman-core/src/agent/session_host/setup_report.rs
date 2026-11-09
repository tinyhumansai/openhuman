//! Reports the steps a turn runs before its first model request to the host's
//! observer (`TurnObservation::Setup`), so a host can see which step cost the
//! time. Reporting is a no-op when no observer is scoped around the turn.

use std::future::Future;
use std::time::Instant;

use tinyagents_harness::observability::turn_observer::{report_setup, SetupOutcome, SetupPhase};

/// Runs `work`, then reports `phase` with how long it took and what
/// `outcome_of` makes of its result.
pub(crate) async fn reported<T>(
    phase: SetupPhase,
    work: impl Future<Output = T>,
    outcome_of: impl FnOnce(&T) -> SetupOutcome,
) -> T {
    let started = Instant::now();
    let result = work.await;
    report_setup(phase, started.elapsed(), outcome_of(&result));
    result
}

/// `Completed` for `Ok`, `Failed` for `Err`.
pub(crate) fn result_outcome<T, E>(result: &Result<T, E>) -> SetupOutcome {
    if result.is_ok() {
        SetupOutcome::Completed
    } else {
        SetupOutcome::Failed
    }
}

#[cfg(test)]
#[path = "setup_report_tests.rs"]
mod tests;
