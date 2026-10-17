//! Autonomous skill-run overrides.
//!
//! `skills_run` runs the orchestrator (and any sub-agents it spawns) as an
//! unattended background tree: it isn't approval-gated (background turns carry
//! no chat approval context), and the per-agent iteration cap is lifted so the
//! run continues until it's done or the repeated-failure circuit breaker trips.
//!
//! The lifted cap rides `OpenHumanRunContext::autonomous_iter_cap`, set by
//! `Agent::set_autonomous_iter_cap` and inherited by every child run context,
//! so one switch covers the whole tree.

/// Lift a sub-agent's per-agent iteration `base` to the active autonomous cap
/// when one is in scope (issue #4463).
///
/// Autonomous skill runs scope an
/// `Agent::set_autonomous_iter_cap` of `TASK_RUN_MAX_ITERATIONS` /
/// `WORKFLOW_RUN_MAX_ITERATIONS` around the whole tree so an unattended run
/// continues until it's done or a circuit breaker trips, rather than stopping at
/// a specialist sub-agent's normal cap (e.g. 10). The migration to the tinyagents
/// harness dropped every reader of `autonomous_iter_cap`, so those setters
/// became dead knobs and sub-agents silently reverted to the normal cap. This is
/// the restored reader: sub-agent iteration computations run their
/// `effective_max_iterations()` through it so the lift takes effect again. The
/// cost budget + repeated-failure breakers remain the primary runaway guards.
pub fn subagent_iter_cap_with_autonomous_lift(autonomous_cap: Option<usize>, base: usize) -> usize {
    match autonomous_cap {
        Some(cap) if cap > base => {
            tracing::debug!(
                base,
                autonomous_cap = cap,
                "[subagent_host:autonomous] lifting sub-agent iteration cap for autonomous run"
            );
            cap
        }
        _ => base,
    }
}

/// A definition's iteration cap under the run context's autonomous lift.
pub fn definition_iter_cap(
    run_context: &crate::agent::tinyagents::host::OpenHumanRunContext,
    definition: &crate::agent::harness::host_definition::AgentDefinition,
) -> usize {
    subagent_iter_cap_with_autonomous_lift(
        run_context.autonomous_iter_cap,
        definition.effective_max_iterations(),
    )
}
