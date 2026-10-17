//! Host accounting and result projection for the harness-owned stop chain.

use std::sync::Arc;
use tinyagents_harness::cost::TurnCost;
use tinyagents_harness::middleware::library::StopHookMiddleware;
use tinyagents_harness::middleware::StopHook;
use tinyagents_harness::steering::SteeringHandle;

/// Install product policies and preserve the host-facing halt summary.
pub(super) fn install(
    harness: &mut tinyagents_harness::runtime::AgentHarness<(), super::host::OpenHumanRunContext>,
    handle: Option<&SteeringHandle>,
    model: &str,
    hooks: Vec<Arc<dyn StopHook<TurnCost>>>,
    halt_summary: tinyagents_harness::turn_outcome::HaltSummarySlot,
) {
    if handle.is_some() && !hooks.is_empty() {
        harness.push_middleware(Arc::new(
            StopHookMiddleware::new(model, hooks)
                .with_metrics_seed(crate::agent::cost::turn_cost())
                .with_stop_observer(move |report| {
                    tracing::warn!(
                        target: "stop_hooks",
                        hook = %report.hook,
                        iteration = report.iteration,
                        model = %report.model,
                        "[stop_hooks] hook voted to stop the turn — pausing run: {}", report.reason
                    );
                    record_halt_summary(&halt_summary, report);
                }),
        ));
    }
}

fn record_halt_summary(
    halt_summary: &tinyagents_harness::turn_outcome::HaltSummarySlot,
    report: &tinyagents_harness::middleware::library::StopReport,
) {
    *halt_summary
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!(
        "Stopping after {} model call(s): {}",
        report.iteration, report.reason
    ));
}

#[cfg(test)]
#[path = "stop_hooks_tests.rs"]
mod tests;
