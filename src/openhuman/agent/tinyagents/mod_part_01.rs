
pub(crate) use crate::openhuman::agent::message_convert::chat_message_to_message;
#[cfg(feature = "flows")]
pub(crate) use crate::openhuman::agent::message_convert::{
    reasoning_from_content, ta_call_to_oh_call,
};
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use tinyagents_harness::agent_loop::AgentStreamItem;
use tinyagents_harness::cache::InMemoryResponseCache;
use tinyagents_harness::context::{RunConfig, RunContext};
use tinyagents_harness::events::EventSink;
use tinyagents_harness::middleware::{
    BudgetLimits, BudgetMiddleware, ContextCompressionMiddleware, PromptCacheGuardMiddleware,
    ToolPolicyMiddleware as TaToolPolicyMiddleware,
};
use tinyinference::model::CapabilitySet;
use tinyagents_harness::retry::RetryPolicy;
use tinyagents_harness::runtime::{AgentHarness, InvalidArgsPolicy, RunPolicy, UnknownToolPolicy};
use tinyagents_harness::steering::SteeringHandle;
use tinyagents_harness::store::StoreRegistry;
use tinyagents_harness::workspace::WorkspaceDescriptor;
use tinyagents_registry::{
    CapabilityRegistry, ComponentKind, DiagnosticSeverity, RegistryDiagnostic, RegistrySnapshot,
};

use crate::openhuman::agent::harness::tool_result_artifacts::{
    ToolResultArtifactIndexStore, TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE,
};
use crate::openhuman::agent::harness::{run_queue::RunQueue, MAX_SPAWN_DEPTH};
use crate::openhuman::agent::messages::{ChatMessage, ConversationMessage};
use crate::openhuman::agent::progress::AgentProgress;

#[allow(unused_imports)] // Wired into the recall/retrieval facade in workstream 09.2.
pub(crate) use embeddings::ProviderEmbeddingModel;
pub(crate) use middleware::{HandoffConfig, TranscriptSnapshotSink, TurnContextMiddleware};
use model::{
    BuiltTurnModels, ProfileOverrideModel, RouteRecordingModel, TierRoutes, TurnChatModel,
};
pub(crate) use observability::SubagentScope;
use observability::{
    CapPauser, IterationCursor, OpenhumanEventBridge, ProviderUsageCarry, ToolFailureMap,
    ToolNameMap,
};
pub use resolved_route::{
    current_resolved_provider_route, current_route_slot, record_resolved_provider_route,
    with_resolved_provider_route_scope, with_route_slot, ResolvedProviderRoute, RouteSlot,
};
pub(crate) use run_cancellation_context::{current_run_cancellation, with_run_cancellation};
#[cfg(test)]
use tools::ToolAdapter;
use tools::{EarlyExitHook, SharedToolAdapter};
pub(crate) use topology::all_graph_topologies;

use std::collections::HashSet;
use std::sync::Arc as StdArc;
use tokio::sync::mpsc::Sender;

/// The builder-configured [`ToolPolicy`](crate::openhuman::agent::tool_policy::ToolPolicy)
/// plus the session context a policy check needs, handed to the shared turn seam
/// so it can install the [`ToolPolicyMiddleware`](middleware::ToolPolicyMiddleware).
/// `None` means "no policy enforcement on this turn" (the channel/CLI + sub-agent
/// paths, which carry their own gating).
pub(crate) struct ToolPolicyEnforcement {
    pub policy: StdArc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    /// The session's channel-permission snapshot — enforces the per-channel
    /// permission ceiling (deny + per-call permission-level gate) the in-house
    /// engine ran in `agent_tool_exec`.
    pub session: crate::openhuman::tools::agent_policy::ToolPolicySession,
    pub session_id: String,
    pub channel: String,
    pub agent_definition_id: String,
}

/// Build the harness [`RunPolicy`] for an openhuman turn.
///
/// The loop enforces limits from `self.policy.limits` (not the per-run
/// `RunConfig`), so the model-call cap **must** be set here or it falls back to
/// the tinyagents default of 25 — far more than openhuman's `max_iterations`.
/// The recursion depth cap is also set here so TinyAgents uses OpenHuman's
/// existing sub-agent spawn depth instead of the SDK default.
/// Retry is now owned by the crate [`RetryPolicy`] (issue #4249, Phase 3a): the
/// turn path no longer wraps its provider in `ReliableProvider` (removed in
/// `session/builder/factory.rs`), so the single retry layer is here, at the
/// harness model call. The schedule mirrors the former `ReliableProvider`
/// defaults — 2 retries (3 attempts) with 500 ms exponential backoff — so
/// transient 429/5xx behavior is preserved. Retryability is decided by the crate
/// `is_retryable`, which the [`native model adapter`](super::model) adapter feeds
/// correctly: a permanent config/auth/quota/context error is mapped to a
/// non-retryable `TinyAgentsError::Validation`, a transient blip to a retryable
/// `Model` error. The crate caps `max_attempts` at
/// `RunLimits::max_retries_per_call + 1` (default 3 retries), so this stays
/// within the loop's own bound.
///
/// (Config parity note: the former `config.reliability.provider_retries` /
/// `provider_backoff_ms` / `model_fallbacks` no longer drive the turn path —
/// retry is the fixed schedule below and cross-route fallback is the crate
/// registry `FallbackPolicy` from [`routes::route_fallback_policy`]. Those config
/// knobs still apply to the non-seam `ReliableProvider` paths.)
///
/// Cross-route **fallback** (`RunPolicy.fallback`) is orthogonal to retry and is
/// populated per-turn by the caller ([`assemble_turn_harness`] via
/// [`routes::route_fallback_policy`]); it is safe to enable now because
/// `ReliableProvider` does *not* fail over across the registered workload-tier
/// routes (chat→burst, reasoning→agentic, …) the way the harness registry can.
/// Default per-turn wall-clock ceiling for an openhuman agent turn, in seconds
/// (issue #4746). Applied as the harness `RunLimits::max_wall_clock_ms` so the
/// loop interrupts a call instead of parking forever with no terminal event.
///
/// 60 minutes (#5766, was 10). With hang detection now owned by the per-call
/// ceiling below, this is a pure runaway guard over the whole turn — its old
/// 600s value doubled as the per-call bound (every call got the turn's
/// *remainder*), which killed long *productive* turns: a turn that had
/// legitimately spent ~543s across many successful calls handed its next model
/// call a 56s budget and died. A turn's real bounds are the model/tool call
/// caps times the per-call ceiling; this only catches what escapes those.
const DEFAULT_AGENT_TURN_TIMEOUT_SECS: u64 = 3_600;

/// Default wall-clock ceiling for a **single model call** within a turn, in
/// seconds (#5766). Applied as the harness `RunLimits::max_model_call_ms`, so
/// every model call (and every retry attempt) gets a fresh
/// `min(this, turn remainder)` budget instead of only the shrinking remainder.
/// This is the hang detector the turn ceiling used to double as — scoped to
/// the one call that wedged, not the whole turn's history.
///
/// Deliberately generous — 15 minutes: a hidden-reasoning model call can be
/// legitimately app-silent for several minutes, so this is a backstop for
/// calls that will never return, not a latency target. Tool calls (including
/// sub-agent delegations, which wrap entire child turns) are exempt by design
/// in the harness and stay bounded by the turn remainder plus their own
/// per-tool timeouts.
const DEFAULT_MODEL_CALL_TIMEOUT_SECS: u64 = 900;

/// Resolve the per-turn wall-clock ceiling in milliseconds for the harness
/// policy. Reads `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` (falling back to
/// [`DEFAULT_AGENT_TURN_TIMEOUT_SECS`]); `0` means "no ceiling" → `None`, which
/// restores the previous unbounded behavior for callers that deliberately opt
/// out (e.g. very long autonomous runs).
pub(crate) fn agent_turn_wall_clock_ms() -> Option<u64> {
    parse_agent_turn_wall_clock_ms(
        std::env::var("OPENHUMAN_AGENT_TURN_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`agent_turn_wall_clock_ms`]: map an optional
/// `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` value to a wall-clock ceiling in
/// milliseconds. An absent/unparseable value falls back to
/// [`DEFAULT_AGENT_TURN_TIMEOUT_SECS`]; `0` yields `None` (unbounded opt-out).
/// Kept env-free so it is deterministically unit-testable.
fn parse_agent_turn_wall_clock_ms(env_value: Option<&str>) -> Option<u64> {
    let secs = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_AGENT_TURN_TIMEOUT_SECS);
    (secs > 0).then(|| secs.saturating_mul(1_000))
}

/// Resolve the per-model-call wall-clock ceiling in milliseconds for the
/// harness policy. Reads `OPENHUMAN_MODEL_CALL_TIMEOUT_SECS` (falling back to
/// [`DEFAULT_MODEL_CALL_TIMEOUT_SECS`]); `0` means "no per-call ceiling" →
/// `None`, leaving calls bounded only by the turn's remaining wall clock as
/// before #5766.
fn model_call_wall_clock_ms() -> Option<u64> {
    parse_model_call_wall_clock_ms(
        std::env::var("OPENHUMAN_MODEL_CALL_TIMEOUT_SECS")
            .ok()
            .as_deref(),
    )
}

/// Pure core of [`model_call_wall_clock_ms`]: map an optional
/// `OPENHUMAN_MODEL_CALL_TIMEOUT_SECS` value to a per-call ceiling in
/// milliseconds. An absent/unparseable value falls back to
/// [`DEFAULT_MODEL_CALL_TIMEOUT_SECS`]; `0` yields `None` (opt-out). Kept
/// env-free so it is deterministically unit-testable — the same shape as
/// [`parse_agent_turn_wall_clock_ms`].
fn parse_model_call_wall_clock_ms(env_value: Option<&str>) -> Option<u64> {
    let secs = env_value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MODEL_CALL_TIMEOUT_SECS);
    (secs > 0).then(|| secs.saturating_mul(1_000))
}

fn run_policy_for(max_iterations: usize, response_cache_enabled: bool) -> RunPolicy {
    let mut policy = RunPolicy::default();
    policy.limits.max_model_calls = max_iterations;
    policy.limits.max_tool_calls = max_iterations.saturating_mul(8).max(8);
    policy.limits.max_depth = MAX_SPAWN_DEPTH;
    // Wall-clock ceiling for the whole turn (issue #4746). The harness bounds
    // every individual model AND tool call by the run's *remaining* wall-clock
    // budget (`with_call_budget` → `tokio::time::timeout`), but ONLY when a
    // deadline is configured — with `max_wall_clock_ms = None` (the default)
    // `call_budget()` returns `None` and each call is awaited UNBOUNDED. That is
    // exactly how a turn shipped an empty reply: a hung/slow model stream or a
    // delegated sub-agent tool call that never returned left the loop parked
    // inside an await, so the between-call deadline check never ran and no
    // terminal event (loop `Timeout` → chat_error) ever fired. Setting the cap
    // here arms the harness's per-call timeout so a wedged call is interrupted
    // mid-flight and the turn degrades gracefully. It also bounds sub-agents:
    // the parent's remaining-budget wraps the sub-agent tool call, and a child
    // turn with no per-run timeout inherits this policy-level cap. Generous by
    // design (a backstop, not a UX deadline); env-overridable, `0` disables.
    policy.limits.max_wall_clock_ms = agent_turn_wall_clock_ms();
    // Per-model-call ceiling (#5766): each model call (and retry attempt) gets
    // a fresh `min(ceiling, turn remainder)` budget, so hang detection is
    // per-call instead of riding the turn deadline — which let the turn
    // ceiling above grow from 10 to 60 minutes without a wedged call being
    // able to hold a turn for more than this. Tool calls (incl. sub-agent
    // delegations) are exempt in the harness and keep the remainder-only
    // budget. Env-overridable, `0` disables.
    policy.limits.max_model_call_ms = model_call_wall_clock_ms();
    // Crate-owned retry (Phase 3a): mirror the former `ReliableProvider` schedule
    // (2 retries, 500 ms exponential backoff). `backoff_sleep` is on so a
    // transient 429/5xx actually waits before retrying, as it did before.
    policy.retry = RetryPolicy {
        max_attempts: 3,
        initial_backoff_ms: 500,
        max_backoff_ms: 30_000,
        multiplier: 2.0,
        jitter: false,
        backoff_sleep: true,
        max_retry_after_ms: RetryPolicy::DEFAULT_MAX_RETRY_AFTER_MS,
        retry_on: None,
    };
    // Unknown-tool recovery (01.2 / C3): the crate policy owns this end to end —
    // the `__openhuman_unknown_tool__` sentinel tool + `UnknownToolRewriteMiddleware`
    // were already deleted. We deliberately keep `ReturnToolError` rather than
    // `Rewrite { tool_name }`: Rewrite requires a real catch-all target tool (the
    // deleted sentinel was exactly that) and, when it hits, *silently* executes
    // that tool and emits `AgentEvent::UnknownToolCall { recovery: "rewrite:.." }`
    // WITHOUT injecting a tool message. `ReturnToolError` instead injects a
    // recoverable `unknown tool `<name>` (arguments: ..); valid tools: [..]`
    // result naming the originally-requested tool. Two live consumers depend on
    // that message: (1) the #4419 attempted-tool-name UX and (2) the failure
    // classifier in `agent::hooks::sanitize_tool_output`, which labels the result
    // `unknown_tool` by matching the "unknown tool" substring. Flipping to Rewrite
    // would drop both. The original name + args are also preserved verbatim on
    // `AgentEvent::UnknownToolCall` and projected by `OpenhumanEventBridge`.
    policy.unknown_tool = UnknownToolPolicy::ReturnToolError;
    // Registered tools with schema-invalid arguments should produce a tool
    // error the model can correct, not abort the entire run. TinyAgents 2.1
    // owns this admission behavior directly; the former host SchemaGuard had
    // to manufacture valid stub arguments only because this policy was left at
    // its historical fail-fast default.
    policy.invalid_args = InvalidArgsPolicy::ReturnToolError;
    // Prompt-prefix protection is always on (issue #4249, 03.2): the
    // `PromptCacheGuardMiddleware` records a `CacheLayoutEvent` whenever volatile
    // content busts the provider KV-cache prefix. Purely diagnostic — never
    // mutates the request.
    policy.cache.protect_prompt_prefix = true;
    // Response caching is gated: it is enabled only for deterministic internal
    // runs (which additionally attach a `ResponseCache`). Interactive chat turns
    // pass `false` here AND attach no cache, so a live user turn can never be
    // served a cached model response (double fail-safe).
    policy.cache.response_cache_enabled = response_cache_enabled;
    // Payload capture ON: the loop stamps request messages + completion onto
    // `ModelCompleted` and tool arguments + result onto `ToolCompleted`, which
    // the `OpenhumanEventBridge` projects into content-bearing `AgentProgress`
    // events (generation/tool span input+output in trace exports). Privacy
    // posture is unchanged off-device: the durable journal passes through a
    // `RedactingSink` (on-device, same data class as the threads DB, which
    // already persists full conversations + tool output), and the Langfuse
    // exporter withholds all content unless
    // `observability.agent_tracing.capture_content` is on.
    policy.capture = tinyagents_harness::runtime::PayloadCapture::all();
    policy
}

/// Consecutive identical tool failures that trip the repeated-failure circuit
/// breaker (see `middleware::RepeatedToolFailureMiddleware`). Three matches the
/// legacy progress-guard's tolerance before it halted a stuck loop.
const REPEATED_TOOL_FAILURE_THRESHOLD: usize = 3;

/// Legacy default model-call cap used when a caller passes `max_iterations == 0`
/// to request "unset" (native-bus / test callers relied on the old loop treating
/// `max_tool_iterations == 0` as the default of 10). Passing `0` straight through
/// would set the harness `max_model_calls` to zero and abort before the first
/// provider call, so the runners normalize `0` to this value.
const DEFAULT_MAX_ITERATIONS: usize = 10;

/// Normalize a caller-supplied iteration cap: `0` means "unset" → the default.
fn effective_max_iterations(max_iterations: usize) -> usize {
    if max_iterations == 0 {
        DEFAULT_MAX_ITERATIONS
    } else {
        max_iterations
    }
}

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
    /// `true` when [`FinalCallWrapUpMiddleware`] turned this turn's last
    /// permitted model call into its conclusion (issue #6014) — the tools were
    /// withdrawn and the wrap-up instruction appended in the loop, so `text`
    /// already **is** the capped turn's answer.
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

/// Shared sink the [`ToolOutcomeCaptureMiddleware`](middleware::ToolOutcomeCaptureMiddleware)
/// appends each tool call's outcome to, drained into the turn outcome.
pub(crate) type ToolOutcomeSink = std::sync::Arc<std::sync::Mutex<Vec<ToolCallOutcome>>>;

/// Shared slot the repeated-failure breaker writes a root-cause halt summary into
/// when it trips. The turn overrides its final text with this summary so the
/// no-progress halt surfaces the cause instead of an empty/last-model reply
/// (legacy `RepeatFailureGuard` parity).
pub(crate) type HaltSummarySlot = std::sync::Arc<std::sync::Mutex<Option<String>>>;

/// Drive an agent turn through the `tinyagents` agent-loop harness.
///
/// Registers `provider` as the default model and every entry in `resolved_tools`
/// as a harness tool, seeds the loop with `history`, and runs the loop bounded
/// by `max_iterations` model calls. Returns the final text plus the resulting
/// transcript translated back to openhuman [`ChatMessage`]s.
#[cfg(test)]
pub(crate) async fn run_turn_via_tinyagents(
    chat_model: TurnChatModel,
    model: &str,
    temperature: f64,
    history: Vec<ChatMessage>,
    resolved_tools: Vec<Arc<dyn crate::openhuman::tools::Tool>>,
    max_iterations: usize,
) -> Result<TinyagentsTurnOutcome> {
    // `0` means "unset" → the legacy default; otherwise the harness cap would be
    // zero and the run would abort before the first model call.
    let max_iterations = effective_max_iterations(max_iterations);
    let mut harness: AgentHarness<()> = AgentHarness::new();
    // Thin test variant: no response cache (chat-safe default).
    harness.with_policy(run_policy_for(max_iterations, false));
    let profile = chat_model.profile().cloned().unwrap_or_default();
    let chat_model: TurnChatModel = Arc::new(
        ProfileOverrideModel::new(chat_model, profile)
            .with_request_model(model)
            .with_request_temperature(temperature),
    );
    let error_slot = Arc::new(std::sync::Mutex::new(None));
    harness
        .register_model(model, chat_model)
        .set_default_model(model);
    let tool_count = resolved_tools.len();
    for tool in resolved_tools {
        harness.register_tool(Arc::new(ToolAdapter::new(tool)));
    }

    // Bound the run: one model call per legacy "iteration", and allow generous
    // tool calls (the loop also stops when the model stops requesting tools).
    let config = RunConfig::new("agent_turn")
        .with_max_model_calls(max_iterations)
        .with_max_tool_calls(max_iterations.saturating_mul(8).max(8))
        .with_max_depth(MAX_SPAWN_DEPTH)
        .with_tag("openhuman")
        .with_tag("scope:root")
        .with_tag("unobserved");

    tracing::info!(
        model,
        max_iterations,
        tools = tool_count,
        "[tinyagents] routing agent turn through tinyagents harness"
    );

    let input = crate::openhuman::agent::message_convert::history_to_messages(&history);
    // Explicit persistence boundary (issue #4455): the request transcript length,
    // captured *before* the run consumes `input`. Everything the harness appends
    // after this index — assistant/tool rounds plus any mid-turn steer messages —
    // is this turn's persisted `conversation`. Anchoring on this index instead of
    // the last-user-message suffix keeps injected steers (which move that
    // boundary) from truncating persisted history.
    let request_base_len = input.len();
    // Box the (large) harness drive future — see `run_turn_via_tinyagents_shared`.
    let run = match Box::pin(harness.invoke(&(), (), config, input)).await {
        Ok(run) => run,
        Err(e) => {
            // #4469 item 3: recover from a poisoned slot instead of panicking.
            // A thread that panicked mid-run while holding this mutex would
            // otherwise turn every subsequent error-recovery read into a second
            // panic, masking the original provider failure. `into_inner` yields
            // the guarded value regardless of poison so we still re-surface the
            // typed error.
            if let Some(original) = error_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                return Err(original);
            }
            return Err(anyhow::anyhow!("tinyagents harness run failed: {e}"));
        }
    };

    let text = run.text().unwrap_or_default();
    let out_history = crate::openhuman::agent::message_convert::messages_to_history(&run.messages);
    let conversation = crate::openhuman::agent::message_convert::messages_to_conversation(
        crate::openhuman::agent::message_convert::messages_since_request(
            &run.messages,
            request_base_len,
        ),
    );
    tracing::debug!(
        request_base_len,
        transcript_len = run.messages.len(),
        persisted_messages = run.messages.len().saturating_sub(request_base_len),
        "[tinyagents] persisting post-request transcript (thin path; steer-safe boundary)"
    );

    Ok(TinyagentsTurnOutcome {
        text,
        history: out_history,
        conversation,
        model_calls: run.model_calls,
        tool_calls: run.tool_calls,
        input_tokens: run.usage.usage.input_tokens,
        output_tokens: run.usage.usage.output_tokens,
        cached_input_tokens: run.usage.usage.cache_read_tokens,
        charged_amount_usd: crate::openhuman::platform::cost::catalog::estimate_cost_usd(
            model,
            run.usage.usage.input_tokens,
            run.usage.usage.output_tokens,
            run.usage.usage.cache_read_tokens,
        ),
        early_exit_tool: None,
        hit_cap: false,
        // The thin (test-only) variant installs no middleware, so nothing could
        // have injected a conclusion.
        wrap_up_injected: false,
        // This thin (test-only) variant does not install the breaker middleware.
        breaker_halt: None,
        // This thin variant carries no per-call outcome capture middleware.
        tool_outcomes: Vec::new(),
    })
}

/// Drive a turn through the tinyagents harness over the routes' **shared**,
/// `Arc`-owned tool registry sets (`Arc<Vec<Box<dyn Tool>>>`), advertising
/// exactly `specs` (already filtered/deduped by the caller's visibility rules).
///
/// This is the entry point the channel/sub-agent routes use to retire the
/// in-house `live` turn machine: it registers a [`SharedToolAdapter`] per
/// advertised spec so the same `Arc`-shared tools the legacy loop runs are
/// reused without cloning.
///
/// `allowed` is the callable tool-name whitelist. Its semantics are
/// **fail-closed** (issue #4452): `None` means "no filter supplied" → every tool
/// visible in `tool_sets` is registered; `Some(set)` registers *exactly* the
/// named tools, so `Some(empty)` is an explicit **deny-all** (zero tools). This
/// distinction is what stops a tool-less sub-agent (`ToolScope::Named([])`, a
/// zero-match `skill_filter`, or a `named` list that resolves to nothing) from
/// silently inheriting the parent's full tool surface (shell/file-write/spawn).
/// Each registered tool is advertised via its own `spec()`.
///
/// When `on_progress` is `Some`, the run streams (`invoke_stream_in_context`)
/// and a [`OpenhumanEventBridge`] mirrors the harness event stream onto
/// `AgentProgress` (live tool timeline, text deltas, cost/token footer) and the
/// global cost tracker — restoring the seams the legacy `run_turn_engine`
/// produced. Pass `None` for fire-and-forget turns (channel/sub-agent) that
/// only need the final text.
///
/// When `context_window` is known, an
/// [`ImageAwareMessageTrimMiddleware`](middleware::ImageAwareMessageTrimMiddleware)
/// keeps history under budget (autocompaction parity).
///
/// `run_queue` forwards mid-flight steer messages into the run; `subagent_scope`
/// re-scopes progress to the `Subagent*` variants (child runs); `early_exit_tools`
/// name the tools that pause the loop (e.g. `ask_user_clarification`) and surface
/// the question via [`TinyagentsTurnOutcome::early_exit_tool`].
/// True when `name` is a sub-agent spawn/delegation tool that a **child** run
/// must never be able to invoke (issue #4452), re-asserted at registration as
/// defense-in-depth so a misconfigured allowlist cannot reintroduce sub-agent
/// spawning into a nested run. Kept local to this seam (rather than importing
/// the `pub(super)` runner helper) so the invariant travels with the
/// registration site that enforces it.
///
/// **This is a strict subset of the caller-side strip**, not a mirror of it
/// (issue #6157). `subagent_runner::tool_prep::is_subagent_spawn_tool` also
/// resolves each archetype's `delegate_name` override through the definition
/// registry — `plan`, `research`, `run_code`, `review_code`, … — none of which
/// carry the `delegate_` prefix this match relies on. Matching them here would
/// put a registry lookup on the per-tool registration loop, so the caller
/// stays responsible for the override names: every path that feeds `allowed`
/// runs `is_subagent_spawn_tool` first, including the dynamic per-spawn tools
/// (`subagent_runner::ops::runner`). Widen this predicate in lockstep if that
/// ever stops being true.
fn is_subagent_spawn_or_delegate_tool(name: &str) -> bool {
    name == "spawn_subagent"
        || name.starts_with("delegate_")
        || name == "agent_prepare_context"
        || name == "spawn_worker_thread"
}
