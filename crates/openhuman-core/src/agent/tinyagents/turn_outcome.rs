//! The outcome type a `tinyagents`-driven turn produces, plus the shared
//! sinks middleware write into to build it.

use tinyagents_harness::turn_outcome::ToolCallOutcome;
use tinyagents_session::transcript::TranscriptMessage;
use tinyinference_llm::model::ResolvedModelRoute;
use tinytools_agent::dialect::TranscriptEntry;

/// The outcome of a turn driven on the `tinyagents` harness.
#[derive(Debug, Clone)]
pub(crate) struct TinyagentsTurnOutcome {
    /// Final assistant text.
    pub text: String,
    /// Concrete provider/model/host route selected for the final successful
    /// model call. This travels explicitly to channel and bus callers; it is
    /// never recovered from a task-local after the run.
    pub resolved_route: Option<ResolvedModelRoute>,
    /// The full transcript, converted back to openhuman messages (flat — tool
    /// calls rendered as text).
    pub history: Vec<TranscriptMessage>,
    /// The **typed** messages this turn appended (after the user turn):
    /// `AssistantToolCalls` / `ToolResults` / final assistant `Chat`. The chat
    /// session persists these to keep structured tool-call history fidelity.
    pub conversation: Vec<TranscriptEntry>,
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
    /// The turn's cost: each call's reported charge, else its catalog
    /// estimate, else unknown (see [`crate::agent::cost::call_cost`]).
    pub cost: tinyagents_harness::cost::CostTally,
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
    /// `true` when the run ended on a reply that ran out of output tokens
    /// (`finish_reason = length`) with no visible text and no tool call — the
    /// model spent its whole output budget reasoning, even after the harness's
    /// truncated-empty retries and nudge (#6951). `text` is then blank, and
    /// the closing call must say the budget ran out rather than claim the
    /// model finished using tools.
    pub truncated: bool,
    /// Per-tool-call execution outcomes (success + raw result content), keyed by
    /// provider call id, captured at the tool boundary. The harness folds a tool
    /// result into a `Message::tool` that drops its `error` flag, so this is the
    /// only place the caller can recover whether each call actually failed — used
    /// to build honest `ToolCallRecord`s for post-turn hooks + the cap checkpoint.
    pub tool_outcomes: Vec<ToolCallOutcome>,
    /// The run's context compaction, when it compacted: re-applied by the
    /// session driver to the history it persists, so the next turn starts
    /// from the checkpoint. `None` when the run did not compact.
    pub compaction: Option<tinyagents_harness::summarization::CompactionCarry>,
}

/// Feed an **unobserved** turn's aggregate usage into the global cost tracker.
///
/// The per-call tracker feed lives in the event bridge
/// ([`OpenhumanEventBridge::record_usage`](super::observability::OpenhumanEventBridge)),
/// which only exists on observed runs (`on_progress` set). Without this
/// aggregate record a fire-and-forget turn's spend never reaches the cost
/// dashboard / wallet surfaces (issue #4249, Phase 5 rollup gap). The bridge
/// and this fallback are mutually exclusive, and the host budget gate does not
/// write the ledger, so spend is recorded exactly once either way.
///
/// Returns `true` when a record was attempted (any tokens observed); all-zero
/// usage is skipped so providers that echo no usage don't inflate the request
/// count. Recording is best-effort — a missing/uninitialised tracker is a
/// silent no-op by contract.
pub(crate) fn record_unobserved_turn_usage(
    ctx: &crate::core::runtime::DispatchCtx,
    model: &str,
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    estimated_usd: Option<f64>,
) -> bool {
    if input_tokens == 0 && output_tokens == 0 {
        return false;
    }
    tracing::debug!(
        model,
        input_tokens,
        output_tokens,
        ?estimated_usd,
        "[tinyagents] recording unobserved-turn usage into the global cost tracker"
    );
    let usage = crate::inference::provider::BilledUsage::from_counts(input_tokens, output_tokens)
        .with_cached_input_tokens(cached_input_tokens);
    // No per-call charge reached this path; record the catalog estimate when
    // the model has one, else the record stays unpriced.
    let usage = match estimated_usd {
        Some(usd) => usage.with_estimated_usd(usd),
        None => usage,
    };
    crate::platform::cost::record_provider_usage(ctx, model, &usage);
    true
}

#[cfg(test)]
#[path = "turn_outcome_tests.rs"]
mod tests;
