//! Issue #6952: one spec check before a root orchestrator turn's first final
//! answer stands.
//!
//! The mechanism is the generic
//! [`VerifyBeforeFinishMiddleware`](tinyagents_harness::middleware::VerifyBeforeFinishMiddleware)
//! from tinyagents. This module owns only OpenHuman's policy: which turns get
//! it, what counts as a multi-step task, and the check text.
//!
//! This is an experimental model-quality lever (a hypothesis to A/B on the
//! bench). On Terminal-Bench 4.0 every failure ended voluntarily, inside its
//! budget, after the model's own checks passed without ever testing the
//! deliverable against what the task stated.

use std::sync::Arc;
use std::time::Duration;

use tinyagents_harness::middleware::{FinishActivity, VerifyBeforeFinishMiddleware};
use tinyagents_harness::runtime::AgentHarness;

use crate::agent::session_host::turn_checkpoint::wrap_harness_instruction;

/// The only agent whose root turns are checked.
const ORCHESTRATOR_AGENT_ID: &str = "orchestrator";

/// Tool rounds after which a turn counts as a multi-step task.
pub(super) const MIN_TOOL_ROUNDS: usize = 5;

/// The session todo tool. Writing a list marks the turn as multi-step on its
/// own, whatever its round count.
pub(super) const TODO_TOOL: &str = "todo";

/// Opening words of the check, also how tests recognise it on a transcript.
pub(super) const CHECK_MARKER: &str = "Before finishing";

const CHECK_INSTRUCTION: &str = "Before finishing: re-read the original request. For each \
explicit requirement, filter, rule or threshold it states, cite evidence from the final \
environment that it holds. Tests count only if derived from the spec, not from your \
implementation. If you flagged an interpretation that contradicts a stated rule, apply the \
literal rule. Fix anything that fails; if all holds, give your final answer in full, since it \
replaces your previous reply.";

/// Whether a turn gets the check: root (not delegated) orchestrator turns only.
/// A sub-agent's answer goes back to its parent, which gets the check on its own
/// answer, so checking every level would multiply the cost for nothing.
pub(super) fn applies(is_subagent: bool, agent_definition_id: Option<&str>) -> bool {
    !is_subagent && agent_definition_id == Some(ORCHESTRATOR_AGENT_ID)
}

/// Whether the turn's activity makes it a multi-step task worth checking.
pub(super) fn should_check(activity: &FinishActivity) -> bool {
    activity.tool_rounds >= MIN_TOOL_ROUNDS || activity.called(TODO_TOOL)
}

/// The check, framed as harness text rather than a user message.
pub(super) fn check_message() -> String {
    wrap_harness_instruction(CHECK_INSTRUCTION)
}

/// Install the check on `harness` when [`applies`] says so. The policy-level
/// turn wall clock is declared to the middleware because it cannot read
/// `RunPolicy` from the run context.
pub(super) fn install<C: Send + Sync + 'static>(
    harness: &mut AgentHarness<(), C>,
    is_subagent: bool,
    agent_definition_id: Option<&str>,
) {
    if !applies(is_subagent, agent_definition_id) {
        tracing::debug!(
            is_subagent,
            agent = agent_definition_id.unwrap_or("<none>"),
            "[verify_before_finish] not installed for this turn"
        );
        return;
    }
    let mut middleware =
        VerifyBeforeFinishMiddleware::new(check_message()).with_trigger(should_check);
    if let Some(ms) = super::agent_turn_wall_clock_ms() {
        middleware = middleware.with_wall_clock_limit(Duration::from_millis(ms));
    }
    tracing::debug!(
        min_tool_rounds = MIN_TOOL_ROUNDS,
        "[verify_before_finish] installed for a root orchestrator turn"
    );
    harness.push_middleware(Arc::new(middleware));
}

#[cfg(test)]
#[path = "verify_before_finish_tests.rs"]
mod tests;
