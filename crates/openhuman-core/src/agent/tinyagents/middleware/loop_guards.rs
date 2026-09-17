//! Loop-guard thresholds, classifiers, and halt-summary wording shared by the
//! repeated-failure and repeat-progress breakers (issue #4463), ported verbatim
//! from the deleted `agent/harness/tool_loop.rs`.

// ── Loop-guard restorations (issue #4463) ────────────────────────────────────
//
// The TinyAgents migration dropped several loop breakers that the crate does not
// replace (verified against `harness::no_progress`, which tracks *failures*
// only): the recoverable-failure headroom, the terminal delegated-inference
// fast-halt (#3104), the policy-denied fast-trip, and the successful-repeat /
// identical-output guards (#4088 / #4095). These helpers + the
// [`RepeatProgressMiddleware`] below restore that behaviour seam-side, ported
// verbatim from the deleted `agent/harness/tool_loop.rs` thresholds/wording so
// the guards read identically to the legacy loop.

/// Recoverable/transient failures get more identical-retry headroom than the
/// deterministic default: a flaky network call or a timeout can succeed on a
/// later attempt once the model adapts (longer timeout, smaller batch, retry).
/// Mirrors the legacy `RECOVERABLE_REPEAT_FAILURE_THRESHOLD`.
pub(crate) const RECOVERABLE_REPEAT_FAILURE_THRESHOLD: u32 = 8;
/// Recoverable failures also get a larger *consecutive* (varied-args) no-progress
/// headroom before the breaker halts. Mirrors the legacy
/// `RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD`.
pub(crate) const RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD: u32 = 12;

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

/// Failures that are informative and plausibly recoverable by changing the next
/// action (longer timeout, smaller batch, different network retry/fallback)
/// rather than by abandoning the turn. Deliberately marker-based and
/// conservative: it only controls breaker headroom, never converts a failure
/// into success. Ported verbatim from legacy `tool_loop::is_recoverable_tool_failure`.
pub(crate) fn is_recoverable_tool_failure(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    [
        "timed out",
        "timeout",
        "deadline exceeded",
        "temporarily unavailable",
        "temporary failure",
        "connection reset",
        "connection refused",
        "connection closed",
        "connection aborted",
        "network is unreachable",
        "host is unreachable",
        "dns error",
        "failed to lookup address",
        "failed to resolve",
        "rate limit",
        "too many requests",
        "retry after",
        "503 service unavailable",
        "502 bad gateway",
        "504 gateway timeout",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
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
    use crate::inference::provider::{
        is_budget_exhausted_message, is_provider_config_rejection_message,
    };
    if !has_inference_failure_envelope(result) {
        return None;
    }
    if is_budget_exhausted_message(result) {
        Some(TerminalInferenceFailure::BudgetExhausted)
    } else if is_provider_config_rejection_message(result) {
        Some(TerminalInferenceFailure::ProviderConfig)
    } else {
        None
    }
}

/// Classify terminal failures at a trusted tool boundary. Delegated inference
/// retains its envelope requirement; first-party media generation and managed
/// web-search tools expose the same provider failure directly, so their
/// canonical tool name supplies the trust boundary.
pub(crate) fn terminal_tool_failure_kind(
    tool: &str,
    result: &str,
) -> Option<TerminalInferenceFailure> {
    if let Some(kind) = terminal_inference_failure_kind(result) {
        return Some(kind);
    }
    if !matches!(
        tool,
        "media_generate_image" | "media_generate_video" | "web_search_tool"
    ) {
        return None;
    }
    use crate::inference::provider::{
        is_budget_exhausted_message, is_provider_config_rejection_message,
    };
    if is_budget_exhausted_message(result) {
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

/// Tools whose contract is to be re-invoked with identical arguments, so an
/// identical repeat is legitimate progress — not a no-progress loop. Today this
/// is `wait_subagent`, which polls a running async sub-agent and explicitly tells
/// the model to "call wait_subagent again" when a `timeout_secs` window elapses
/// while the sub-agent is still running. Without this exemption a task that
/// outlives two wait windows would have its third identical `wait_subagent`
/// halted by the no-progress breakers before it could collect the eventual
/// result. Ported from legacy `tool_loop::is_repeat_call_exempt` (Codex P1 on #4230).
pub(crate) fn is_repeat_call_exempt(tool: &str) -> bool {
    matches!(tool, "wait_subagent")
}
