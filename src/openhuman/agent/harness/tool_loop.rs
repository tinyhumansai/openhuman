use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, Provider};
use crate::openhuman::tools::policy::{DefaultToolPolicy, ToolPolicy};
use crate::openhuman::tools::Tool;
use anyhow::Result;
use std::collections::HashSet;

use super::payload_summarizer::PayloadSummarizer;

/// Minimum characters per chunk when relaying LLM text to a streaming draft.
pub(crate) const STREAM_CHUNK_MIN_CHARS: usize = 80;

/// Default maximum agentic tool-use iterations per user message to prevent runaway loops.
/// Used as a safe fallback when `max_tool_iterations` is unset or configured as zero.
pub(crate) const DEFAULT_MAX_TOOL_ITERATIONS: usize = 10;

/// Extended iteration cap for agents with `IterationPolicy::Extended`. These
/// are multi-step specialists (code executor, integrations, planner, …) whose
/// realistic workflows commonly exceed the default 10-iteration cap. The
/// repeated-failure circuit breaker and cost budget remain the primary runaway
/// guards; this value is intentionally generous to avoid premature stops.
pub(crate) const EXTENDED_MAX_TOOL_ITERATIONS: usize = 50;

/// Repeated-failure circuit breaker. The plain iteration cap lets an agent grind
/// the same dead-end (e.g. re-running `pip install` when there is no pip) until
/// `max_iterations`, then return an opaque `MaxIterationsExceeded` that the caller
/// just re-spawns — losing the failure context. These thresholds let the loop bail
/// EARLY with a root-cause summary instead.
///
/// If the SAME `(tool, args)` call fails this many times, the agent is repeating a
/// known-failed action verbatim — stop.
pub(crate) const REPEAT_FAILURE_THRESHOLD: u32 = 3;
/// Recoverable/transient failures (timeouts, connection resets, rate limits, ...)
/// are still bounded, but need more room than deterministic terminal failures so
/// the model can adapt (change timeout, narrow work, split a command, retry a
/// flaky network call) before the breaker stops the turn.
pub(crate) const RECOVERABLE_REPEAT_FAILURE_THRESHOLD: u32 = 8;
/// If this many non-recoverable tool calls fail back-to-back with no success in
/// between (even with varied args), the agent is making no progress — stop.
pub(crate) const NO_PROGRESS_FAILURE_THRESHOLD: u32 = 6;
/// Recoverable failures get a separate, larger no-progress headroom. The
/// iteration cap and cost budget still bound the turn, while a handful of
/// timeouts no longer stops an otherwise adaptable agent.
pub(crate) const RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD: u32 = 12;
/// Hard policy rejections (a security block or a gate denial) are deterministic:
/// the identical `(tool, args)` call provably cannot succeed. Halt on the FIRST
/// verbatim repeat — i.e. the second identical attempt — rather than letting the
/// agent burn the generic [`REPEAT_FAILURE_THRESHOLD`] on a doomed call. The first
/// occurrence is allowed through so the model can read the "do not retry" reason
/// and pivot to a different, allowed approach.
pub(crate) const HARD_REJECT_REPEAT_THRESHOLD: u32 = 2;

/// Classification of a deterministic, recognizable policy rejection, detected via
/// the stable markers the security/approval layers emit
/// ([`crate::openhuman::security::POLICY_BLOCKED_MARKER`] /
/// [`POLICY_DENIED_MARKER`](crate::openhuman::security::POLICY_DENIED_MARKER)).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum HardReject {
    /// Permanent for this tier (read-only write, forbidden/credential path,
    /// disallowed command) — never succeeds on retry.
    Blocked,
    /// User denied / approval timed out this turn — re-asking the identical call
    /// only re-prompts.
    Denied,
}

/// Recognize a hard policy rejection from a tool result. Matches anywhere in the
/// string (not just the prefix) so it survives the `Error: …` wrapping the tool
/// layer adds. `Blocked` takes precedence over `Denied` if both somehow appear.
pub(crate) fn hard_reject_kind(result: &str) -> Option<HardReject> {
    if result.contains(crate::openhuman::security::POLICY_BLOCKED_MARKER) {
        Some(HardReject::Blocked)
    } else if result.contains(crate::openhuman::security::POLICY_DENIED_MARKER) {
        Some(HardReject::Denied)
    } else {
        None
    }
}

/// A permanent, non-retryable inference failure surfaced by a tool result —
/// typically a delegated sub-agent (`run_code` / `tools_agent` / `plan`) whose
/// provider call hit a user-state wall. Unlike a transient error, re-issuing the
/// call cannot succeed even under a *different* delegation tool or varied args:
/// the budget is account-wide and the model/provider configuration is shared by
/// every sub-agent. See [`terminal_inference_failure_kind`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TerminalInferenceFailure {
    /// Out of inference budget / credits — every retry hits the same wall.
    /// Detected via
    /// [`is_budget_exhausted_message`](crate::openhuman::inference::provider::is_budget_exhausted_message).
    BudgetExhausted,
    /// The configured model/provider rejected the request for a reason the user
    /// must fix (unknown model, non-chat/embedding model, missing credential,
    /// region block, …). Detected via
    /// [`is_provider_config_rejection_message`](crate::openhuman::inference::provider::is_provider_config_rejection_message).
    ProviderConfig,
}

/// Inference/delegation **envelope** markers that prove a tool result came from
/// a delegated inference call (a sub-agent / provider round-trip) rather than
/// from arbitrary tool stderr.
///
/// The two provider classifiers ([`is_budget_exhausted_message`] /
/// [`is_provider_config_rejection_message`]) match on short message substrings
/// (`"insufficient balance"`, `"invalid temperature"`, `"model field is
/// required"`, …) that can legitimately appear in a *recoverable* tool's output
/// — e.g. a `shell`/`run_code` script printing `ValueError: invalid temperature`
/// or a test asserting on `"model field is required"`. Applying the terminal
/// halt to those would misreport a fixable script failure as "fix the model or
/// API key" and stop after a single attempt.
///
/// Gating on these envelope markers scopes the classifier to genuinely
/// delegated inference failures. Every marker here is **harness-generated** —
/// produced by our own reliable-chain rollup or sub-agent dispatch wrapper, NOT
/// by a provider HTTP body that arbitrary tool stderr could forge:
///   * the reliable-chain exhaustion rollup (`"All providers/models failed"` /
///     `"may not be available on your provider"`, reliable.rs), and
///   * the sub-agent dispatch wrapper (`"failed and did not complete"`, see
///     [`crate::openhuman::agent_orchestration::tools::dispatch::format_subagent_failure`]).
///
/// **Why the bare provider envelope is NOT a marker (Codex review #3779):** the
/// raw provider-HTTP shape (`"<provider> API error (…)"`, `"<provider> Responses
/// API error: …"`) is reproducible verbatim by a *recoverable* tool that is
/// debugging its own API client — e.g. a `shell`/`run_code` script printing
/// `OpenAI API error (400): invalid temperature` or `… model field is required`.
/// Matching the bare `"api error"` substring there would let that script trip
/// the broad provider-config classifier and HALT the whole turn after a single
/// failed command with a misleading "fix your model in Settings → AI" message,
/// even though the agent should just recover. Every *genuine* delegated
/// inference failure additionally surfaces through one of the harness wrappers
/// above (a sub-agent provider error reaches the orchestrator only via
/// `dispatch::format_subagent_failure`; a direct reliable-chain exhaustion via
/// the rollup), so dropping the bare provider envelope loses no real detection
/// while closing the false-positive on tool stderr.
///
/// [`is_budget_exhausted_message`]: crate::openhuman::inference::provider::is_budget_exhausted_message
/// [`is_provider_config_rejection_message`]: crate::openhuman::inference::provider::is_provider_config_rejection_message
const INFERENCE_FAILURE_ENVELOPE_MARKERS: &[&str] = &[
    // Reliable-chain exhaustion rollup (reliable.rs::format_failure_aggregate).
    "all providers/models failed",
    "may not be available on your provider",
    // Sub-agent delegation failure wrapper (dispatch.rs::format_subagent_failure).
    "failed and did not complete",
];

/// True if `result` carries one of the inference/delegation envelope markers —
/// i.e. the failure demonstrably came from a delegated provider round-trip, not
/// from an arbitrary tool's stderr. See [`INFERENCE_FAILURE_ENVELOPE_MARKERS`].
fn has_inference_failure_envelope(result: &str) -> bool {
    let lower = result.to_ascii_lowercase();
    INFERENCE_FAILURE_ENVELOPE_MARKERS
        .iter()
        .any(|marker| lower.contains(marker))
}

/// Recognize a permanent (non-retryable) inference failure from a tool result.
///
/// Two-stage gate so a *recoverable* tool failure can't be misclassified:
///   1. The result must carry a delegated-inference **envelope**
///      ([`has_inference_failure_envelope`]) — proving it came from a sub-agent
///      / provider round-trip and not from arbitrary tool stderr that merely
///      happens to contain a classifier substring (e.g. a `shell` script
///      printing `ValueError: invalid temperature` or a test asserting on
///      `"model field is required"`). Without this guard a fixable script/test
///      failure would be misreported as "fix the model or API key" and stopped
///      after a single attempt (Codex review #3779).
///   2. The (then-trusted) body is matched against the two deliberately-tight
///      provider classifiers, which stay in lockstep with the Sentry-demotion
///      phrase sets: a transient / 5xx / generic 4xx body matches NEITHER, so
///      genuinely retryable failures still get the normal consecutive-failure
///      grace ([`NO_PROGRESS_FAILURE_THRESHOLD`]) and are never halted early.
///      Budget takes precedence if both somehow match.
///
/// The orchestrator otherwise re-emits a failed delegation under *varied* tool
/// names (Plan → `run_code` → `tools_agent`), so the identical-`(tool,args)`
/// [`REPEAT_FAILURE_THRESHOLD`] never trips and the chain grinds through ~6-8
/// doomed, paid delegations before [`NO_PROGRESS_FAILURE_THRESHOLD`] finally
/// halts with an opaque "Something went wrong" (#3104). Tripping on the FIRST
/// permanent failure stops that cascade and surfaces the root cause.
pub(crate) fn terminal_inference_failure_kind(result: &str) -> Option<TerminalInferenceFailure> {
    use crate::openhuman::inference::provider::{
        is_budget_exhausted_message, is_provider_config_rejection_message,
    };
    // Require the delegated-inference envelope first: the message-only
    // classifiers are too broad to apply to arbitrary tool stderr.
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

/// Failures that are informative and plausibly recoverable by changing the next
/// action (longer timeout, smaller batch, different network retry/fallback)
/// rather than by immediately abandoning the turn.
///
/// Keep this deliberately marker-based and conservative: it only controls
/// breaker headroom, never converts a failure into success. Hard policy rejects
/// and permanent provider/account failures are classified before this function.
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

/// Shared repeated-failure circuit breaker, used by BOTH agent loops
/// (`run_tool_call_loop` here and `run_inner_loop` in `subagent_runner`) so they
/// can't drift. Tracks per-`(tool,args)`-signature failure counts and a
/// consecutive-failure run within a single agent turn; [`Self::record`] returns
/// a root-cause halt summary once a threshold trips.
#[derive(Default)]
pub(crate) struct RepeatFailureGuard {
    sig_counts: std::collections::HashMap<String, u32>,
    consecutive: u32,
    consecutive_recoverable: u32,
}

impl RepeatFailureGuard {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record one tool-call outcome. `args_sig` is a stable string form of the
    /// arguments (e.g. the command). Returns `Some(summary)` when the breaker
    /// trips — the caller should stop the loop and return that summary as the
    /// agent's result instead of grinding to `max_iterations`.
    pub(crate) fn record(
        &mut self,
        tool: &str,
        args_sig: &str,
        success: bool,
        result: &str,
    ) -> Option<String> {
        if success {
            self.consecutive = 0;
            self.consecutive_recoverable = 0;
            return None;
        }
        let count = {
            let c = self
                .sig_counts
                .entry(format!("{tool}|{args_sig}"))
                .or_insert(0);
            *c += 1;
            *c
        };
        // Permanent inference failures (out of budget / provider-config rejection)
        // cannot be recovered by retrying — the budget is account-wide and the
        // model/provider config is shared by every (sub-)agent. Halt on the FIRST
        // occurrence with an actionable root cause instead of letting the
        // orchestrator re-emit the step under varied delegation-tool names until
        // NO_PROGRESS_FAILURE_THRESHOLD (#3104: Plan → Run Code ×6 → Tools Agent
        // ×2). Checked before the count-based thresholds precisely because those
        // never trip in time when the tool name keeps changing.
        if let Some(kind) = terminal_inference_failure_kind(result) {
            tracing::warn!(
                tool,
                kind = ?kind,
                "[agent_loop] permanent inference failure — halting on first occurrence with root cause"
            );
            return Some(match kind {
                TerminalInferenceFailure::BudgetExhausted => format!(
                    "Stopping: the `{tool}` step failed because the account is out of inference \
                     budget/credits — every retry hits the same wall. Add credits to your account \
                     (or, when using a custom/BYO provider, top up that provider's own account) \
                     and try again. Details:\n{}",
                    truncate_for_halt(result),
                ),
                TerminalInferenceFailure::ProviderConfig => format!(
                    "Stopping: the `{tool}` step failed because the configured model/provider \
                     rejected the request (e.g. an unknown model, a non-chat/embedding model, a \
                     missing credential, or a region block) — retrying will not help. Fix the model \
                     or API key in Settings → AI. Details:\n{}",
                    truncate_for_halt(result),
                ),
            });
        }
        // Hard policy rejections trip on the first verbatim repeat; recoverable
        // failures get extra headroom; everything else uses the generic
        // identical-retry threshold.
        let hard = hard_reject_kind(result);
        let recoverable = hard.is_none() && is_recoverable_tool_failure(result);
        if recoverable {
            self.consecutive_recoverable += 1;
            tracing::debug!(
                tool,
                count,
                consecutive_recoverable = self.consecutive_recoverable,
                "[agent_loop] recoverable tool failure recorded with extended circuit-breaker headroom"
            );
        } else {
            self.consecutive += 1;
            self.consecutive_recoverable = 0;
        }
        let repeat_threshold = if hard.is_some() {
            HARD_REJECT_REPEAT_THRESHOLD
        } else if recoverable {
            RECOVERABLE_REPEAT_FAILURE_THRESHOLD
        } else {
            REPEAT_FAILURE_THRESHOLD
        };
        if count >= repeat_threshold {
            return Some(match hard {
                Some(HardReject::Blocked) => format!(
                    "Stopping: the `{tool}` call is blocked by the security policy and was \
                     re-issued with identical arguments — it can never succeed this way. \
                     Reason:\n{}\n\nDo not repeat this call; use an allowed alternative or report \
                     that it can't be done here.",
                    truncate_for_halt(result),
                ),
                Some(HardReject::Denied) => format!(
                    "Stopping: the `{tool}` call was denied and re-issued unchanged — re-asking \
                     will not change the answer. Reason:\n{}\n\nDo not repeat this call; take a \
                     different approach or report that it can't be done here.",
                    truncate_for_halt(result),
                ),
                None => format!(
                    "Stopping: the `{tool}` call was retried {count} times with identical \
                     arguments and kept failing — repeating it will not help. Last error:\n{}\n\n\
                     {} Report this back instead of retrying.",
                    truncate_for_halt(result),
                    if recoverable {
                        "This looked recoverable at first, but the same call exhausted the \
                         extended transient-failure headroom."
                    } else {
                        "This looks unrecoverable in the current environment (e.g. a missing \
                         tool/dependency that cannot be installed here)."
                    },
                ),
            });
        }
        if recoverable {
            if self.consecutive_recoverable >= RECOVERABLE_NO_PROGRESS_FAILURE_THRESHOLD {
                return Some(format!(
                    "Stopping: {} recoverable-looking tool failures happened in a row with no \
                     successful progress. Last error (from `{tool}`):\n{}\n\nThe turn is still \
                     bounded by the iteration/cost limits, but this many consecutive transient \
                     failures means the goal is not currently reachable. Report this back instead \
                     of retrying.",
                    self.consecutive_recoverable,
                    truncate_for_halt(result),
                ));
            }
            return None;
        }
        if self.consecutive >= NO_PROGRESS_FAILURE_THRESHOLD {
            return Some(format!(
                "Stopping: {} tool calls in a row failed with no progress. Last error (from \
                 `{tool}`):\n{}\n\nDifferent commands are all failing — the goal looks unreachable \
                 in this environment. Report this back instead of retrying.",
                self.consecutive,
                truncate_for_halt(result),
            ));
        }
        None
    }
}

/// If the model emits the IDENTICAL assistant output (narrative text + the same
/// tool-call name/args) this many times in a row, it's stuck in a no-progress
/// narration loop — halt. Set low enough to bail early (the observed
/// degeneration repeated ~195×) but above any legitimate short retry.
pub(crate) const REPEAT_OUTPUT_THRESHOLD: u32 = 4;

/// Repeat-OUTPUT circuit breaker — distinct from [`RepeatFailureGuard`], which
/// only counts tool *failures* and resets on every success.
///
/// This catches the degenerate case where each iteration re-emits the SAME
/// narration + SAME tool call and the call nominally "succeeds" yet nothing
/// advances (e.g. the model narrating "now let me create the files…" and
/// re-issuing the same `run_code` forever). That loop is invisible to two
/// things people reach for first:
///   * `frequency_penalty` — per-generation only; each iteration is a fresh,
///     individually non-repetitive generation, so it has nothing to penalise
///     and no memory across turns.
///   * [`RepeatFailureGuard`] — resets on success, so a repeated *successful*
///     no-op never trips it.
///
/// Trips on `REPEAT_OUTPUT_THRESHOLD` consecutive identical signatures; a
/// different signature (real progress) resets the run, so interleaved varied
/// work never trips it.
#[derive(Default)]
pub(crate) struct RepeatOutputGuard {
    last_hash: Option<u64>,
    consecutive: u32,
}

impl RepeatOutputGuard {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record one iteration's output signature (assistant text + tool-call
    /// name/args). Returns `Some(halt summary)` once the identical signature has
    /// repeated [`REPEAT_OUTPUT_THRESHOLD`] times back-to-back.
    pub(crate) fn record(&mut self, signature: &str) -> Option<String> {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        signature.hash(&mut hasher);
        let h = hasher.finish();
        if self.last_hash == Some(h) {
            self.consecutive += 1;
        } else {
            self.last_hash = Some(h);
            self.consecutive = 1;
        }
        if self.consecutive >= REPEAT_OUTPUT_THRESHOLD {
            return Some(format!(
                "Stopping: the last {} iterations produced the IDENTICAL response and tool call \
                 with no change — the run is stuck repeating the same step without making \
                 progress. Re-issuing it will not help. Summarise what (if anything) was actually \
                 accomplished and report that the task could not progress, or take a genuinely \
                 different approach.",
                self.consecutive,
            ));
        }
        None
    }
}

/// Clamp the last-error text embedded in a circuit-breaker halt summary so a huge
/// tool error (already capped at 1MB upstream) can't blow up the agent's result.
pub(crate) fn truncate_for_halt(s: &str) -> String {
    const MAX: usize = 600;
    if s.chars().count() <= MAX {
        return s.to_string();
    }
    let head: String = s.chars().take(MAX).collect();
    format!("{head}\n… [truncated]")
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
/// When `silent` is true, suppresses stdout (for channel use).
///
/// This is a thin wrapper around [`run_tool_call_loop`] with the per-agent
/// filter and extra-tool plumbing disabled — i.e. the LLM sees the entire
/// `tools_registry` unchanged. Used by legacy call sites and harness tests
/// that don't need agent-aware scoping.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn agent_turn(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    multimodal_file_config: &crate::openhuman::config::MultimodalFileConfig,
    max_tool_iterations: usize,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
) -> Result<String> {
    let default_policy = DefaultToolPolicy;
    run_tool_call_loop(
        provider,
        history,
        tools_registry,
        provider_name,
        model,
        temperature,
        silent,
        "channel",
        multimodal_config,
        multimodal_file_config,
        max_tool_iterations,
        None,
        None,
        &[],
        None,
        payload_summarizer,
        &default_policy,
    )
    .await
}

/// Execute a single turn of the agent loop: send messages, parse tool calls,
/// execute tools, and loop until the LLM produces a final text response.
///
/// # Per-agent tool scoping
///
/// The last two parameters support per-agent tool filtering without
/// requiring callers to build a filtered copy of the (non-`Clone`able)
/// tool registry:
///
/// * `visible_tool_names` — optional whitelist of tool names that are
///   allowed to reach the LLM. When `Some(set)`, only tools whose
///   `name()` is present in the set contribute to the function-calling
///   schema and are eligible for execution; every other tool in the
///   registry is hidden from the model and rejected if the model
///   somehow emits a call for it. When `None`, no filtering is applied
///   and every tool in the combined registry is visible (the legacy
///   behaviour used by CLI/REPL and harness tests).
///
/// * `extra_tools` — per-turn synthesised tools to splice alongside the
///   persistent `tools_registry`. The agent-dispatch path uses this to
///   surface delegation tools (`research`, `plan`,
///   `delegate_to_integrations_agent`, …) that are synthesised fresh
///   per turn from the active agent's `subagents` field and the
///   current Composio integration list, and therefore are not
///   registered in the global startup-time registry.
///
/// The combined tool list seen by the LLM this turn is
/// `tools_registry.iter().chain(extra_tools.iter())`, further narrowed
/// by `visible_tool_names` when supplied.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool_call_loop(
    provider: &dyn Provider,
    history: &mut Vec<ChatMessage>,
    tools_registry: &[Box<dyn Tool>],
    provider_name: &str,
    model: &str,
    temperature: f64,
    silent: bool,
    // Retained in the harness signature (callers pass their channel) but no
    // longer consumed here since the legacy CLI approval prompt was removed —
    // approval now flows through the process-global `ApprovalGate`.
    _channel_name: &str,
    multimodal_config: &crate::openhuman::config::MultimodalConfig,
    multimodal_file_config: &crate::openhuman::config::MultimodalFileConfig,
    max_tool_iterations: usize,
    on_delta: Option<tokio::sync::mpsc::Sender<String>>,
    visible_tool_names: Option<&HashSet<String>>,
    extra_tools: &[Box<dyn Tool>],
    on_progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    payload_summarizer: Option<&dyn PayloadSummarizer>,
    tool_policy: &dyn ToolPolicy,
) -> Result<String> {
    let max_iterations = if max_tool_iterations == 0 {
        DEFAULT_MAX_TOOL_ITERATIONS
    } else {
        max_tool_iterations
    };

    // The agentic loop itself now lives in the shared turn engine; this
    // function is a thin adapter that builds the channel/CLI tool source
    // (registry + per-turn extras, visibility whitelist, pluggable policy)
    // and hands off. The signature is retained verbatim so existing callers
    // (the `agent.run_turn` bus handler, triage, the payload summarizer, and
    // the harness test suite) are unaffected.
    log::debug!(
        "[tool-loop] Registry has {} tool(s), extra {} tool(s), filter={}",
        tools_registry.len(),
        extra_tools.len(),
        visible_tool_names
            .map(|s| format!("whitelist({})", s.len()))
            .unwrap_or_else(|| "none".to_string()),
    );
    let mut tool_source = super::engine::RegistryToolSource::new(
        tools_registry,
        extra_tools,
        visible_tool_names,
        tool_policy,
        payload_summarizer,
    );
    let progress = super::engine::TurnProgress::new(on_progress);
    let mut observer = super::engine::NullObserver;
    let checkpoint = super::engine::ErrorCheckpoint;
    let parser = super::engine::DefaultParser;
    super::engine::run_turn_engine(
        provider,
        history,
        &mut tool_source,
        &progress,
        &mut observer,
        &checkpoint,
        &parser,
        provider_name,
        model,
        temperature,
        silent,
        multimodal_config,
        multimodal_file_config,
        max_iterations,
        on_delta,
        &[],
        None,
        None, // channel/CLI/triage loop: context guard + token-budget trim only
    )
    .await
    .map(|outcome| outcome.text)
}

#[cfg(test)]
#[path = "tool_loop_tests.rs"]
mod tests;
