//! Loop-guard halt copy: the root-cause summaries a circuit breaker records
//! when a turn is stopped for repeated tool failures, plus the recogniser for a
//! permanent delegated-inference failure that picks between the two terminal
//! summaries. Thresholds and the repeat-call exemption stay with the guards in
//! `agent::tinyagents::middleware::loop_guards`.

use tinyinference_providers::{
    is_budget_message, is_provider_config_rejection_message, BudgetMatch,
};

/// Clamp the last-error text embedded in a circuit-breaker halt summary so a huge
/// tool error (already capped at 1MB upstream) can't blow up the agent's result.
/// Mirrors the legacy `tool_loop::truncate_for_halt`.
pub(crate) fn truncate_for_halt(s: &str) -> String {
    const MAX: usize = 600;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX).collect();
    format!("{head}\n… [truncated]")
}

/// A permanent, non-retryable inference failure surfaced by a delegated
/// sub-agent's tool result. Unlike a transient error, re-issuing the call cannot
/// succeed even under a *different* delegation tool or varied args: the budget is
/// account-wide and the model/provider configuration is shared by every
/// (sub-)agent. See [`terminal_inference_failure_kind`] (#3104).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TerminalInferenceFailure {
    /// Out of inference budget / credits — every retry hits the same wall.
    BudgetExhausted,
    /// The configured model/provider rejected the request for a reason the user
    /// must fix (unknown model, non-chat/embedding model, missing credential,
    /// region block, …).
    ProviderConfig,
}

/// Inference/delegation **envelope** markers that prove a tool result came from a
/// delegated inference call (a sub-agent / provider round-trip) rather than from
/// arbitrary tool stderr. Every marker here is harness-generated (our own
/// reliable-chain rollup or sub-agent dispatch wrapper), NOT a provider HTTP body
/// that arbitrary tool stderr could forge. Ported from legacy `tool_loop`.
const INFERENCE_FAILURE_ENVELOPE_MARKERS: &[&str] = &[
    // Reliable-chain exhaustion rollup (reliable.rs::format_failure_aggregate).
    "all providers/models failed",
    "may not be available on your provider",
    // Sub-agent delegation failure wrapper (dispatch.rs::format_subagent_failure).
    "failed and did not complete",
];

/// True if `result` carries one of the inference/delegation envelope markers —
/// i.e. the failure demonstrably came from a delegated provider round-trip, not
/// arbitrary tool stderr. See [`INFERENCE_FAILURE_ENVELOPE_MARKERS`].
fn has_inference_failure_envelope(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    INFERENCE_FAILURE_ENVELOPE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Recognize a permanent (non-retryable) delegated-inference failure from a tool
/// result. Two-stage gate so a *recoverable* tool failure can't be misclassified:
/// (1) the result must carry a delegated-inference envelope
/// ([`has_inference_failure_envelope`]); (2) the trusted body is matched against
/// the two tight provider classifiers. Budget takes precedence if both match.
/// Ported from legacy `tool_loop::terminal_inference_failure_kind` (#3104).
pub(crate) fn terminal_inference_failure_kind(result: &str) -> Option<TerminalInferenceFailure> {
    if !has_inference_failure_envelope(result) {
        return None;
    }
    if is_budget_message(result, BudgetMatch::Billing) {
        Some(TerminalInferenceFailure::BudgetExhausted)
    } else if is_provider_config_rejection_message(result) {
        Some(TerminalInferenceFailure::ProviderConfig)
    } else {
        None
    }
}

/// The actionable root-cause halt summary for a terminal delegated-inference
/// failure. Ported verbatim from the legacy loop.
pub(crate) fn terminal_inference_halt_summary(
    kind: TerminalInferenceFailure,
    tool: &str,
    result: &str,
) -> String {
    match kind {
        TerminalInferenceFailure::BudgetExhausted => format!(
            "Stopping: the `{tool}` step failed because the account is out of inference \
             budget/credits — every retry hits the same wall. Add credits to your account \
             (or, when using a custom/BYO provider, top up that provider's own account) and try \
             again. Details:\n{}",
            truncate_for_halt(result),
        ),
        TerminalInferenceFailure::ProviderConfig => format!(
            "Stopping: the `{tool}` step failed because the configured model/provider rejected the \
             request (e.g. an unknown model, a non-chat/embedding model, a missing credential, or \
             a region block) — retrying will not help. Fix the model or API key in Connections → API keys → LLM. \
             Details:\n{}",
            truncate_for_halt(result),
        ),
    }
}

/// Halt summary when a single recoverable `(tool, args)` call exhausts its
/// extended identical-retry headroom. Ported from the legacy loop.
pub(crate) fn recoverable_identical_halt_summary(tool: &str, count: u32, result: &str) -> String {
    format!(
        "Stopping: the `{tool}` call was retried {count} times with identical arguments and kept \
         failing — repeating it will not help. Last error:\n{}\n\nThis looked recoverable at \
         first, but the same call exhausted the extended transient-failure headroom. Report this \
         back instead of retrying.",
        truncate_for_halt(result),
    )
}

/// Halt summary when many recoverable-looking failures pile up with no progress.
/// Ported from the legacy loop.
pub(crate) fn recoverable_no_progress_halt_summary(
    consecutive: u32,
    tool: &str,
    result: &str,
) -> String {
    format!(
        "Stopping: {consecutive} recoverable-looking tool failures happened in a row with no \
         successful progress. Last error (from `{tool}`):\n{}\n\nThe turn is still bounded by the \
         iteration/cost limits, but this many consecutive transient failures means the goal is not \
         currently reachable. Report this back instead of retrying.",
        truncate_for_halt(result),
    )
}

/// Recognise a **user-actionable** blocker in a failing tool result — one only
/// the user can clear — and phrase the halt as a direct ask instead of the
/// crate's generic "the goal looks unreachable in this environment, report this
/// back" summary (issue #4092). Today that's a missing service connection (the
/// issue's canonical example: acting on a service that isn't connected). Such a
/// failure will never self-resolve by retrying, and the fix is the user's, so
/// escalate with a concrete next step instead of looping or reporting a generic
/// dead-end. Returns `None` for failures that are not user-actionable, leaving
/// the crate's summary in place.
pub(crate) fn user_actionable_escalation(tool: &str, error: &str) -> Option<String> {
    let lower = error.to_lowercase();
    let permission_or_scope_failure = lower.contains("[composio:error:insufficient_scope]")
        || lower.contains("[composio:error:trigger_permission]")
        || lower.contains("insufficient scope")
        || lower.contains("insufficient authentication scopes")
        || lower.contains("insufficient permissions")
        || lower.contains("missing required permissions")
        || lower.contains("permission to manage triggers");
    if permission_or_scope_failure {
        return None;
    }
    // Keep this narrow: some scope/permission failures legitimately tell the
    // user to reconnect in Connections, but they are not missing connections.
    let missing_connection = lower.contains("[composio:error:composio_platform]")
        || lower.contains("not connected")
        || lower.contains("isn't connected")
        || lower.contains("is not connected")
        || lower.contains("not enabled")
        || lower.contains("token revoked")
        || lower.contains("connection error, try to authenticate");
    if !missing_connection {
        return None;
    }
    Some(format!(
        "I can't continue without your input: the `{tool}` action needs a service that isn't \
         connected. {}\n\nConnect it (Connections), then tell me to retry — or \
         tell me how you'd like to proceed instead.",
        crate::util::truncate_with_ellipsis(error, 400),
    ))
}

#[cfg(test)]
#[path = "halt_tests.rs"]
mod tests;
