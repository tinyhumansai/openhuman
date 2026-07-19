//! Autonomous skill-run overrides.
//!
//! `skills_run` runs the orchestrator (and any sub-agents it spawns) as an
//! unattended background tree: it isn't approval-gated (background turns carry
//! no `APPROVAL_CHAT_CONTEXT`), and the per-agent iteration cap is lifted so the
//! run continues until it's done or the repeated-failure circuit breaker trips.
//!
//! The lifted cap rides a `tokio` task-local set around the orchestrator's
//! `run_single`. Sub-agent inner loops are awaited *inline* within that scope
//! (`run_subagent` does not detach), so the task-local reaches them too — one
//! switch covers the whole tree.

use std::future::Future;

tokio::task_local! {
    static AUTONOMOUS_ITER_CAP: usize;
}

/// The active autonomous iteration cap, if a skill run scoped one.
pub fn autonomous_iter_cap() -> Option<usize> {
    AUTONOMOUS_ITER_CAP.try_with(|c| *c).ok()
}

/// Run `fut` with an autonomous iteration cap in scope. The cap propagates to
/// every agentic loop awaited within — the orchestrator turn and the inline
/// sub-agent loops.
pub async fn with_autonomous_iter_cap<F: Future>(cap: usize, fut: F) -> F::Output {
    AUTONOMOUS_ITER_CAP.scope(cap, fut).await
}

/// Lift a sub-agent's per-agent iteration `base` to the active autonomous cap
/// when one is in scope (issue #4463).
///
/// Autonomous task/skill runs (`task_dispatcher` / `skill_runtime`) scope an
/// [`with_autonomous_iter_cap`] of `TASK_RUN_MAX_ITERATIONS` /
/// `WORKFLOW_RUN_MAX_ITERATIONS` around the whole tree so an unattended run
/// continues until it's done or a circuit breaker trips, rather than stopping at
/// a specialist sub-agent's normal cap (e.g. 10). The migration to the tinyagents
/// harness dropped every reader of [`autonomous_iter_cap`], so those setters
/// became dead knobs and sub-agents silently reverted to the normal cap. This is
/// the restored reader: sub-agent iteration computations run their
/// `effective_max_iterations()` through it so the lift takes effect again. The
/// cost budget + repeated-failure breakers remain the primary runaway guards.
pub fn subagent_iter_cap_with_autonomous_lift(base: usize) -> usize {
    match autonomous_iter_cap() {
        Some(cap) if cap > base => {
            tracing::debug!(
                base,
                autonomous_cap = cap,
                "[subagent_runner:autonomous] lifting sub-agent iteration cap for autonomous run"
            );
            cap
        }
        _ => base,
    }
}
