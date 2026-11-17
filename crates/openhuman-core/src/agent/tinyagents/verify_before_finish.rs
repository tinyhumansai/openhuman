//! Issues #6952 and #6990: one requirements check before a root orchestrator
//! turn's first final answer stands.
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

use tinyagents_harness::middleware::{
    FinalCallWrapUpMiddleware, FinishActivity, VerifyBeforeFinishMiddleware,
};
use tinyagents_harness::runtime::AgentHarness;

use tinyagents_harness::middleware::wrap_harness_instruction;

/// The only agent whose root turns are checked.
const ORCHESTRATOR_AGENT_ID: &str = "orchestrator";

/// Tool rounds after which a turn counts as a multi-step task.
pub(super) const MIN_TOOL_ROUNDS: usize = 5;

/// The session todo tool. Writing a list marks the turn as multi-step on its
/// own, whatever its round count.
pub(super) const TODO_TOOL: &str = "todo";

/// Opening words of the check, also how tests recognise it on a transcript.
pub(super) const CHECK_MARKER: &str = "Before finishing";

const CHECK_INSTRUCTION: &str = "Before finishing: re-read the original request and check \
the final result against it.\n\
1. Split the task statement into individual requirements. Include every constraint, \
prohibited case, expected output or error, threshold, and case in parentheses.\n\
2. For each requirement, name and perform an independent check using the exact examples, \
inputs and conditions stated in the request. Verify the complete outcome, including \
expected errors and side effects, and cite evidence from the final environment. Checks \
count only if derived from the request, not from your own approach.\n\
3. Where a sentence allows two readings, examine both against the surrounding requirements. \
Apply the literal rule where it resolves the ambiguity; where both readings can be satisfied \
at once, make the result satisfy both and test each. Where they cannot (one path called a \
folder and a file), let the evidence decide in this order: the request's own test, verifier \
or example call; the form of the value it shows (a value with a file extension is being used \
as a file); the prose label last. Otherwise check both readings and state any remaining \
uncertainty instead of treating a check of your guess as proof.\n\
4. Find equivalent paths or workflows that perform the same action and verify that their \
required outcomes, notifications and state changes are consistent.\n\
5. Where the result will face inputs you have not seen (a hidden test set, \"all\" or \"any\" \
inputs, other files of the same kind), build several fresh inputs that differ from the \
example in the ways the request allows, adversarial ones for anything that filters or \
validates, and run them. Where an oracle, reference tool or ground truth is available, \
check every part of the result against it, not a sample. A property you never varied is \
one you have not checked, so do not record it as verified.\n\
6. Run the final check through the exact interface the request names, from the state as it \
will be judged: the exact path, file name, command line, port or format, and the exact \
allowed or forbidden elements where a list is given. A check through a copied helper, a \
scratch launcher, a different entry point or a metric of your own is a debugging signal, \
not proof. Where the request states a number or a reference to meet, measure against that \
and state the measured value. Once every check the contract names passes, the named tests, \
verifier script or reference tool included, that state is the result: stop exploring and \
stop polishing. One passing check through the real interface is not the whole contract; a \
named test still failing or never run means you are not done.\n\
7. Leave the result as requested, not as your checks left it: remove what your checks \
created that the request did not ask for (build outputs, compiled binaries, scratch and \
test files), especially next to the deliverable, then confirm that what remains is exactly \
what was asked for, no more. The same for actions: an irreversible change the request did \
not ask for (rewriting history, deleting or resetting data, changing settings) is not an \
improvement; if you believe one is warranted, say so in your answer instead of making it.\n\n\
Fix anything that fails and re-run the affected checks. If all holds, give your final \
answer in full, since it replaces your previous reply.";

/// Whether a turn gets the check: root (not delegated) orchestrator turns only.
/// A sub-agent's answer goes back to its parent, which gets the check on its own
/// answer, so checking every level would multiply the cost for nothing.
pub(super) fn applies(is_subagent: bool, agent_definition_id: Option<&str>) -> bool {
    !is_subagent && agent_definition_id == Some(ORCHESTRATOR_AGENT_ID)
}

/// Whether the turn's activity makes it a multi-step task worth checking.
pub(super) fn should_check(activity: &FinishActivity) -> bool {
    FinishActivity::rounds_or_called(MIN_TOOL_ROUNDS, TODO_TOOL)(activity)
}

/// The check, framed as harness text rather than a user message.
pub(super) fn check_message() -> String {
    wrap_harness_instruction(CHECK_INSTRUCTION)
}

/// Install the check on `harness` when [`applies`] says so. The policy-level
/// turn wall clock is declared to the middleware because it cannot read
/// `RunPolicy` from the run context.
///
/// `wrap_up` must be the same `Arc` that is installed as the turn's wrap-up
/// middleware (`with_wrap_up` only sees announcements made by that instance).
/// Once it has announced a budget notice ("N calls left, finish now") the check
/// stays quiet, so the two directives cannot contradict each other (tinyagents#301).
pub(super) fn install<C: Send + Sync + 'static>(
    harness: &mut AgentHarness<(), C>,
    is_subagent: bool,
    agent_definition_id: Option<&str>,
    wrap_up: &Option<Arc<FinalCallWrapUpMiddleware>>,
    turn_wall_clock_ms: Option<u64>,
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
    if let Some(ms) = turn_wall_clock_ms {
        middleware = middleware.with_wall_clock_limit(Duration::from_millis(ms));
    }
    if let Some(wrap_up) = wrap_up.as_ref() {
        middleware = middleware.with_wrap_up(Arc::clone(wrap_up));
    }
    tracing::debug!(
        min_tool_rounds = MIN_TOOL_ROUNDS,
        wrap_up_linked = wrap_up.is_some(),
        "[verify_before_finish] installed for a root orchestrator turn"
    );
    harness.push_middleware(Arc::new(middleware));
}

#[cfg(test)]
#[path = "verify_before_finish_tests.rs"]
mod tests;
