//! The outcome type a `tinyagents`-driven turn produces, plus the shared
//! sinks middleware write into to build it.

use crate::agent::messages::{ChatMessage, ConversationMessage};

/// The outcome of a turn driven on the `tinyagents` harness.
#[derive(Debug, Clone)]
pub(crate) struct TinyagentsTurnOutcome {
    /// Final assistant text.
    pub text: String,
    /// The full transcript, converted back to openhuman messages (flat — tool
    /// calls rendered as text).
    pub history: Vec<ChatMessage>,
    /// The **typed** messages this turn appended (after the user turn):
    /// `AssistantToolCalls` / `ToolResults` / final assistant `Chat`. The chat
    /// session persists these to keep structured tool-call history fidelity.
    pub conversation: Vec<ConversationMessage>,
    /// Number of model calls the loop made.
    pub model_calls: usize,
    /// Number of tool calls the loop made.
    pub tool_calls: usize,
    /// Accumulated input tokens.
    pub input_tokens: u64,
    /// Accumulated output tokens.
    pub output_tokens: u64,
    /// Accumulated cached (cache-read) input tokens. Carried so the turn persists
    /// real cached usage instead of zero (issue #4249, Phase 5).
    pub cached_input_tokens: u64,
    /// Input tokens in the most recent primary model call. This is deliberately
    /// not accumulated across iterations: it represents one context window.
    pub last_call_input_tokens: u64,
    /// Output tokens in the most recent primary model call.
    pub last_call_output_tokens: u64,
    /// Estimated charged USD for the turn (from `cost::catalog::estimate_cost_usd`
    /// over the observed usage). Carried so the transcript / session meters record
    /// a real cost instead of `$0` on every non-cap turn.
    pub charged_amount_usd: f64,
    /// Set when an early-exit tool (e.g. `ask_user_clarification`) fired: the
    /// loop paused so the caller can checkpoint and surface the question. When
    /// present, `text` holds the question. Mirrors the legacy `early_exit_tool`.
    pub early_exit_tool: Option<String>,
    /// `true` when the run stopped because it reached the model-call cap with
    /// work still pending (the last response requested more tools). The caller
    /// should summarize a resumable checkpoint rather than treat `text` as a
    /// final answer — the tinyagents analogue of the legacy cap checkpoint seam.
    pub hit_cap: bool,
    /// `true` when [`FinalCallWrapUpMiddleware`](super::middleware::FinalCallWrapUpMiddleware)
    /// turned this turn's last permitted model call into its conclusion (issue
    /// #6014) — the tools were withdrawn and the wrap-up instruction appended in
    /// the loop, so `text` already **is** the capped turn's answer.
    ///
    /// Read alongside [`hit_cap`](Self::hit_cap) rather than folded into it,
    /// because the caller's action differs: with this set there is nothing left
    /// to ask the model for, while a cap reached without it (a run with the
    /// middleware uninstalled, or one whose final call still came back empty)
    /// keeps the out-of-band `summarize_turn_wrapup` path it always had. That
    /// makes the in-loop conclusion strictly additive — no path loses the
    /// behaviour it has today.
    pub wrap_up_injected: bool,
    /// Set (with the root-cause halt summary) when the repeated-tool-failure /
    /// repeat-progress circuit breaker halted the run before a natural finish.
    /// The sub-agent runner surfaces this as `SubagentRunStatus::Incomplete`
    /// (#4466) so a parent does NOT treat a halted child as a clean completion.
    /// `text` already carries this same summary; the flag lets the status mapper
    /// distinguish a breaker halt from a genuine final answer.
    pub breaker_halt: Option<String>,
    /// Per-tool-call execution outcomes (success + raw result content), keyed by
    /// provider call id, captured at the tool boundary. The harness folds a tool
    /// result into a `Message::tool` that drops its `error` flag, so this is the
    /// only place the caller can recover whether each call actually failed — used
    /// to build honest `ToolCallRecord`s for post-turn hooks + the cap checkpoint.
    pub tool_outcomes: Vec<ToolCallOutcome>,
}

/// One tool call's execution outcome, captured at the tool boundary before the
/// harness discards the failure flag. `success` mirrors the absence of a
/// `TaToolResult::error`; `content` is the (possibly summarized/capped) result
/// text used to derive a sanitized post-turn summary.
#[derive(Debug, Clone)]
pub(crate) struct ToolCallOutcome {
    pub call_id: String,
    pub name: String,
    pub success: bool,
    pub content: String,
}

/// Shared sink the [`ToolOutcomeCaptureMiddleware`](super::middleware::ToolOutcomeCaptureMiddleware)
/// appends each tool call's outcome to, drained into the turn outcome.
pub(crate) type ToolOutcomeSink = std::sync::Arc<std::sync::Mutex<Vec<ToolCallOutcome>>>;

/// Shared slot the repeated-failure breaker writes a root-cause halt summary into
/// when it trips. The turn overrides its final text with this summary so the
/// no-progress halt surfaces the cause instead of an empty/last-model reply
/// (legacy `RepeatFailureGuard` parity).
pub(crate) type HaltSummarySlot = std::sync::Arc<std::sync::Mutex<Option<String>>>;

/// Feed an **unobserved** turn's aggregate usage into the global cost tracker.
///
/// The per-call tracker feed lives in the event bridge
/// ([`OpenhumanEventBridge::record_usage`](super::observability::OpenhumanEventBridge)),
/// which only exists on observed runs (`on_progress` set). Without this
/// aggregate record a fire-and-forget turn's spend never reaches the cost
/// dashboard / wallet surfaces (issue #4249, Phase 5 rollup gap). The bridge
/// and this fallback are mutually exclusive, so spend is recorded exactly once
/// either way.
///
/// Returns `true` when a record was attempted (any tokens observed); all-zero
/// usage is skipped so providers that echo no usage don't inflate the request
/// count. Recording is best-effort — a missing/uninitialised tracker is a
/// silent no-op by contract.
pub(crate) fn record_unobserved_turn_usage(
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    charged_amount_usd: f64,
) -> bool {
    if input_tokens == 0 && output_tokens == 0 {
        return false;
    }
    tracing::debug!(
        model,
        input_tokens,
        output_tokens,
        charged_usd = charged_amount_usd,
        "[tinyagents] recording unobserved-turn usage into the global cost tracker"
    );
    crate::platform::cost::record_provider_usage(
        model,
        &crate::inference::provider::UsageInfo {
            input_tokens,
            output_tokens,
            context_window: 0,
            cached_input_tokens,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            charged_amount_usd,
        },
    );
    true
}
