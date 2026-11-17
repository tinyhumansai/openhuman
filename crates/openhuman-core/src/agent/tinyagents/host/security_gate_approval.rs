//! Approval outcomes projected into runtime gate decisions and model text.

use super::{GateDecision, GateOutcome};

/// What the model is told when the approval prompt for `tool_name` expired
/// unanswered. Like the refusal text it carries no `[policy-denied]` marker
/// (see [`decision_for_outcome`]), so the model keeps the turn to tell the
/// user.
fn unanswered_approval_text(tool_name: &str) -> String {
    format!(
        "The approval request for '{tool_name}' was not answered in time, so it was not run. \
         Tell the user the approval window expired and that they must ask again to retry. \
         Do not retry this call yourself this turn and do not achieve the same result another \
         way (shell, CLI, another tool)."
    )
}

/// Maps how the approval flow settled to what the runtime is told.
///
/// A refusal is a `Deny` whose text names no human and no timeout — every
/// refusal reads the same — and rules out other routes explicitly, because
/// `shell` never prompts while autonomy is disabled (`[autonomy] enabled =
/// false`, the default) and was used to redo a refused call.
///
/// The text carries no `[policy-denied]` marker on purpose. The marker
/// classifies as a policy failure with a zero recovery budget
/// (`middleware::repeated_failure`), which pauses the turn before the model can
/// answer. The model must get this turn to tell the user the action was not
/// done, as it did with the harness's unmarked fallback text.
pub(super) fn decision_for_outcome(tool_name: &str, outcome: GateOutcome) -> GateDecision {
    match outcome {
        GateOutcome::Allow => GateDecision::Prompted { approved: true },
        // An expired prompt is not a refusal: nobody answered, and the gate has
        // now denied the request. Saying so lets the agent (and, for a
        // sub-agent, its parent) tell the user the approval window expired and
        // that they must ask again for a new request, rather than that it was
        // declined — the generic text below left a 600s `media_generate_image`
        // expiry reported as nothing at all.
        GateOutcome::Deny { reason }
            if crate::security::approval::is_unanswered_approval_reason(&reason) =>
        {
            tracing::warn!(
                target: "tinyagents",
                tool = %tool_name,
                "[tinyagents::host::security] approval prompt expired unanswered"
            );
            GateDecision::deny(unanswered_approval_text(tool_name))
        }
        GateOutcome::Deny { reason } => {
            tracing::warn!(
                target: "tinyagents",
                tool = %tool_name,
                reason = %reason,
                "[tinyagents::host::security] approval flow declined the tool call"
            );
            GateDecision::deny(
                "This action was refused and must not be performed this turn — do not retry this \
                 call and do not achieve the same result another way (shell, CLI, another tool). \
                 Tell the user it was not done.",
            )
        }
    }
}
