//! Turn-error projection of the harness's typed [`HostedError`].
//!
//! `HostedError::message` is a fixed, sanitized string per kind, so the
//! per-model-call ceiling and an exhausted run budget look identical once the
//! error is flattened. The bound is carried structurally instead
//! (`HostedError::timeout_bound`), and this projection writes it into the turn
//! error with [`TurnTimeoutBound`] so the Sentry `timeout_bound` tag survives
//! the hosted path. The user-facing class is untouched: the text still reads as
//! `run timed out: ...`, which `web_errors` classifies as `turn_timeout`.

use tinyagents_harness::runtime::{HostedError, HostedErrorKind, TimeoutBound};
use tinyagents_harness::TinyAgentsError;

use tinyagents_harness::agent_error::TurnTimeoutBound;

/// The host-side name of a harness [`TimeoutBound`].
pub(super) fn turn_timeout_bound(bound: TimeoutBound) -> TurnTimeoutBound {
    match bound {
        TimeoutBound::PerModelCall => TurnTimeoutBound::PerModelCall,
        // `TimeoutBound` is non-exhaustive; any other bound is a run-level one.
        _ => TurnTimeoutBound::RunRemaining,
    }
}

/// Convert a hosted failure into the run error the turn pipeline maps.
///
/// Identical to `TinyAgentsError::from(error)` except that a timeout names the
/// bound that fired. It stays a terminal `Timeout`, as before.
pub(crate) fn run_error_from_hosted(error: HostedError) -> TinyAgentsError {
    let bound = match (error.kind, error.timeout_bound) {
        (HostedErrorKind::Timeout, Some(bound)) => Some(turn_timeout_bound(bound)),
        _ => None,
    };
    match bound {
        Some(bound) => {
            tracing::debug!(
                timeout_bound = bound.tag(),
                "[tinyagents] hosted turn timed out; carrying the typed bound"
            );
            TinyAgentsError::Timeout(format!(
                "hosted agent invocation exceeded its {}",
                bound.phrase()
            ))
        }
        None => TinyAgentsError::from(error),
    }
}

#[cfg(test)]
#[path = "hosted_error_tests.rs"]
mod tests;
