//! Host adapter over `tinyagents_harness::middleware::library::install_wind_down`.
//!
//! The harness owns the mechanism (pause once the turn's `TurnDeadline`
//! wind-down point passes, clamp `max_wall_clock_ms` to the hard stop). The
//! host contributes only the callback that records the pause on the turn's
//! session sidecar, so the driver reports the turn as wound down rather than
//! completed (`session_host::driver::grounded_close` then writes the answer).

use std::sync::Arc;

use tinyagents_harness::middleware::library::{install_wind_down, WindDownMiddleware};
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::steering::SteeringHandle;

use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::observability::SubagentScope;

/// Install the harness wind-down for a top-level turn that carries a deadline
/// and record its pause on the turn sidecar.
pub(super) fn install(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
    handle: &Option<SteeringHandle>,
    run_context: &OpenHumanRunContext,
    subagent_scope: &Option<SubagentScope>,
) -> Option<Arc<WindDownMiddleware>> {
    let middleware = install_wind_down(
        harness,
        handle.as_ref(),
        run_context.turn_deadline,
        subagent_scope.is_some(),
    )?;
    let sidecar = run_context.session_sidecar.clone();
    middleware.set_on_pause(Arc::new(move || {
        sidecar
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .wind_down = true;
        tracing::debug!(
            target: "turn_deadline",
            "[turn_deadline] wind-down recorded on the turn sidecar; the turn will be reported as stopped"
        );
    }));
    Some(middleware)
}

#[cfg(test)]
#[path = "deadline_wind_down_tests.rs"]
mod tests;
