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
