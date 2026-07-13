//! `tinyagents` integration — drive an openhuman agent turn on the published
//! [`tinyagents`](https://crates.io/crates/tinyagents) orchestration framework
//! (issue #4249).
//!
//! openhuman's agent execution runs on the `tinyagents` crate
//! (LangGraph/LangChain-style durable graphs + an agent-loop harness with model/
//! tool registries, middleware, retry/fallback, and limits). This module is the
//! **adapter seam**: it bridges openhuman's `Provider`, `Tool`, and `ChatMessage`
//! types onto the crate's `ChatModel`, `Tool`, and `Message` traits, then drives
//! a turn through [`AgentHarness::invoke`]. The chat / channel / sub-agent
//! routes call [`run_turn_via_tinyagents_shared`] (default ON in production).
//!
//! The chat route is at functional parity with the legacy `run_turn_engine`:
//! the [`OpenhumanEventBridge`] mirrors the harness event stream onto
//! `AgentProgress` (live tool timeline, incremental text deltas, cost footer),
//! [`ProviderModel::stream`] forwards true token streaming, multimodal markers
//! are expanded, and history is trimmed to the context window. Mid-flight
//! steering, sub-agent child-progress deltas (incl. thinking), and the
//! `ask_user_clarification` early-exit pause are all re-wired onto the
//! tinyagents harness.

mod abort_guard;
mod convert;
pub(crate) mod delegation;
mod embeddings;
pub(crate) mod journal;
pub(crate) mod middleware;
pub(crate) mod model;
pub(crate) mod observability;
pub(crate) mod orchestration;
pub(crate) mod payload_summarizer;
mod policy_denial;
pub(crate) mod replay;
pub(crate) mod retriever;
mod routes;
pub(crate) mod run_cancellation_context;
mod steering_forwarder;
pub(crate) mod stop_hooks;
pub(crate) mod subagent_graph;
mod summarize;
pub(crate) mod tools;
mod topology;

pub(crate) use convert::{
    chat_message_to_message, reasoning_from_content, spec_to_schema, ta_call_to_oh_call,
};

use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use tinyagents::harness::agent_loop::AgentStreamItem;
use tinyagents::harness::cache::InMemoryResponseCache;
use tinyagents::harness::context::{RunConfig, RunContext};
use tinyagents::harness::events::EventSink;
use tinyagents::harness::middleware::{
    BudgetLimits, BudgetMiddleware, ContextCompressionMiddleware, PromptCacheGuardMiddleware,
    ToolPolicyMiddleware as TaToolPolicyMiddleware,
};
use tinyagents::harness::model::CapabilitySet;
use tinyagents::harness::retry::RetryPolicy;
use tinyagents::harness::runtime::{AgentHarness, RunPolicy, UnknownToolPolicy};
use tinyagents::harness::steering::SteeringHandle;
use tinyagents::harness::store::StoreRegistry;
use tinyagents::harness::workspace::WorkspaceDescriptor;
use tinyagents::registry::{
    CapabilityRegistry, ComponentKind, DiagnosticSeverity, RegistryDiagnostic, RegistrySnapshot,
};

use crate::openhuman::agent::harness::tool_result_artifacts::{
    ToolResultArtifactIndexStore, TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE,
};
use crate::openhuman::agent::harness::{run_queue::RunQueue, MAX_SPAWN_DEPTH};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage, Provider};

#[allow(unused_imports)] // Wired into the recall/retrieval facade in workstream 09.2.
pub(crate) use embeddings::ProviderEmbeddingModel;
pub(crate) use middleware::{
    HandoffConfig, SuperContextConfig, TranscriptSnapshotSink, TurnContextMiddleware,
};
use model::ProviderModel;
pub(crate) use observability::SubagentScope;
use observability::{
    CapPauser, IterationCursor, OpenhumanEventBridge, ProviderUsageCarry, ToolFailureMap,
    ToolNameMap,
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
    pub session: crate::openhuman::agent_tool_policy::ToolPolicySession,
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
/// `is_retryable`, which the [`ProviderModel`](super::model) adapter feeds
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
/// loop interrupts a hung/slow model or tool/sub-agent call instead of parking
/// forever with no terminal event. Deliberately generous — a normal turn, even
/// a slow multi-step reasoning one, finishes well under it; this is a hang
/// backstop, not a UX deadline. 10 minutes.
const DEFAULT_AGENT_TURN_TIMEOUT_SECS: u64 = 600;

/// Resolve the per-turn wall-clock ceiling in milliseconds for the harness
/// policy. Reads `OPENHUMAN_AGENT_TURN_TIMEOUT_SECS` (falling back to
/// [`DEFAULT_AGENT_TURN_TIMEOUT_SECS`]); `0` means "no ceiling" → `None`, which
/// restores the previous unbounded behavior for callers that deliberately opt
/// out (e.g. very long autonomous runs).
fn agent_turn_wall_clock_ms() -> Option<u64> {
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
    policy.capture = tinyagents::harness::runtime::PayloadCapture::all();
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
    provider: Arc<dyn Provider>,
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
    let provider_model = ProviderModel::new(provider, model, temperature);
    let error_slot = provider_model.error_slot();
    harness
        .register_model(model, Arc::new(provider_model))
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

    let input = convert::history_to_messages(&history);
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
    let out_history = convert::messages_to_history(&run.messages);
    let conversation = convert::messages_to_conversation(convert::messages_since_request(
        &run.messages,
        request_base_len,
    ));
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
        charged_amount_usd: crate::openhuman::cost::catalog::estimate_cost_usd(
            model,
            run.usage.usage.input_tokens,
            run.usage.usage.output_tokens,
            run.usage.usage.cache_read_tokens,
        ),
        early_exit_tool: None,
        hit_cap: false,
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
/// must never be able to invoke (issue #4452). Mirrors the caller-side strip in
/// `subagent_runner::tool_prep::is_subagent_spawn_tool` plus the worker-thread
/// spawn, re-asserted at registration as defense-in-depth so a misconfigured
/// allowlist cannot reintroduce sub-agent spawning into a nested run. Kept local
/// to this seam (rather than importing the `pub(super)` runner helper) so the
/// invariant travels with the registration site that enforces it.
fn is_subagent_spawn_or_delegate_tool(name: &str) -> bool {
    name == "spawn_subagent"
        || name.starts_with("delegate_")
        || name == "use_tinyplace"
        || name == "agent_prepare_context"
        || name == "spawn_worker_thread"
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_turn_via_tinyagents_shared(
    turn_models: TurnModels,
    provider_id: String,
    model: &str,
    history: Vec<ChatMessage>,
    tool_sets: Vec<Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>>,
    allowed: Option<HashSet<String>>,
    max_iterations: usize,
    on_progress: Option<Sender<AgentProgress>>,
    subagent_scope: Option<SubagentScope>,
    context_window: Option<u64>,
    run_queue: Option<Arc<RunQueue>>,
    early_exit_tools: &[&str],
    pause_at_cap: bool,
    max_output_tokens: Option<u32>,
    context_mw: TurnContextMiddleware,
    tool_policy: Option<ToolPolicyEnforcement>,
    workspace_descriptor: Option<WorkspaceDescriptor>,
    deterministic_cacheable: bool,
    // #4457 (defect C): when `true`, the seam does NOT emit the terminal
    // `TurnCompleted` — the caller emits it itself *after* its post-run wrap-up
    // (e.g. the chat/session path streams a cap/#4093 checkpoint via
    // `summarize_turn_wrapup` after this seam returns, so a seam-level emit here
    // would land `turn_active = false` before that checkpoint finishes
    // streaming, and the web bridge would record two ledger events + two
    // Completed upserts). Callers with no post-run streaming (channel/CLI) pass
    // `false` and rely on this seam's emit for parity with the legacy engine.
    defer_turn_completed_to_caller: bool,
) -> Result<TinyagentsTurnOutcome> {
    // `0` means "unset" → the legacy default (a native-bus / test convention);
    // otherwise the harness model-call cap would be zero and abort the run before
    // the first provider call.
    let max_iterations = effective_max_iterations(max_iterations);
    // The turn's crate `ChatModel` set (`turn_models`) and the provider telemetry
    // id are built by the caller via `build_turn_models` — the seam entry is
    // crate-native and no longer names `Provider` (issue #4249, Phase 5). The
    // telemetry id (`{provider_id}.{model}` in Langfuse) rides in as a param.
    let AssembledTurnHarness {
        harness,
        cursor,
        tool_names,
        failure_map,
        provider_usage_carry,
        error_slot,
        halt_summary,
        tool_outcome_sink,
        handle,
        early_exit_hook,
        tool_count,
        registry_snapshot: _,
        registry_diagnostics,
        tool_result_artifact_index,
        compression_mw,
        prompt_cache_guard,
    } = assemble_turn_harness(
        turn_models,
        model,
        tool_sets,
        allowed,
        max_iterations,
        on_progress.clone(),
        subagent_scope.clone(),
        context_window,
        early_exit_tools,
        context_mw,
        tool_policy,
        routes::turn_required_capabilities(model),
        deterministic_cacheable,
    );

    // Fail-closed registry validation gate (issue #4249, Workstream 10 — registry).
    // The projected `CapabilityRegistry` produced these diagnostics during
    // assembly; enforce them here, *before* the first model dispatch, so an
    // ambiguous/broken tool surface (duplicate name across native/MCP/Composio/
    // generated tools, dangling alias, etc.) aborts the turn instead of silently
    // resolving to an unintended component while a provider call is in flight.
    if !registry_diagnostics.is_empty() {
        let (errors, warnings): (Vec<&RegistryDiagnostic>, Vec<&RegistryDiagnostic>) =
            registry_diagnostics
                .iter()
                .partition(|d| matches!(d.severity, DiagnosticSeverity::Error));
        for diag in &warnings {
            tracing::warn!(
                kind = diag.kind.as_str(),
                name = %diag.name,
                "[registry] non-fatal diagnostic: {}",
                diag.message
            );
        }
        if !errors.is_empty() {
            let messages: Vec<String> = errors
                .iter()
                .map(|d| format!("[{}] {}: {}", d.kind.as_str(), d.name, d.message))
                .collect();
            for msg in &messages {
                tracing::error!("[registry] error-severity diagnostic aborting turn: {msg}");
            }
            tracing::error!(
                error_count = messages.len(),
                warning_count = warnings.len(),
                "[registry] aborting turn before model dispatch: capability registry validation failed"
            );
            return Err(anyhow::Error::new(
                crate::openhuman::agent::error::AgentError::RegistryValidationFailed {
                    diagnostics: messages,
                },
            ));
        }
        tracing::debug!(
            warning_count = warnings.len(),
            "[registry] registry diagnostics present (warnings only); proceeding with turn"
        );
    }

    let mut config = RunConfig::new("agent_turn")
        .with_max_model_calls(max_iterations)
        .with_max_tool_calls(max_iterations.saturating_mul(8).max(8))
        .with_max_depth(MAX_SPAWN_DEPTH)
        .with_tag("openhuman")
        .with_tag(if subagent_scope.is_some() {
            "scope:subagent"
        } else {
            "scope:root"
        })
        .with_tag(if on_progress.is_some() {
            "observed"
        } else {
            "unobserved"
        });
    // Per-turn output cap rides RunConfig now (Phase 5 groundwork): the loop
    // stamps it onto every `ModelRequest.max_tokens` and the ProviderModel
    // adapter honors it, so the cap no longer bakes into the primary + route
    // models. Mirrors the legacy `AGENT_TURN_MAX_OUTPUT_TOKENS` / sub-agent cap.
    if let Some(cap) = max_output_tokens {
        config = config.with_max_turn_output_tokens(cap);
    }

    tracing::info!(
        model,
        max_iterations,
        tools = tool_count,
        observed = on_progress.is_some(),
        "[tinyagents] routing turn through tinyagents harness (shared tools)"
    );

    let input = convert::history_to_messages(&history);
    // Explicit persistence boundary (issue #4455): the request transcript length,
    // captured *before* the run consumes `input`. The turn's persisted
    // `conversation` is everything appended past this index — assistant/tool
    // rounds plus any mid-turn steer/collect messages injected as user turns.
    // Anchoring here (instead of the last-user-message suffix) keeps injected
    // steers from moving the boundary and truncating persisted history on both
    // the parent (`session/turn/core.rs`) and subagent (`subagent_runner`) paths.
    let request_base_len = input.len();

    // Build the run context: an optional event sink feeds the progress/cost
    // bridge (streaming) and/or the model-call-cap pauser; the shared steering
    // handle carries mid-flight, early-exit, and cap pauses.
    let cancellation = tinyagents::CancellationToken::new();
    let mut ctx = RunContext::new(config, ()).with_cancellation(cancellation.clone());
    if let Some(descriptor) = workspace_descriptor {
        tracing::debug!(
            root = %descriptor.root.display(),
            policy_id = %descriptor.policy_id,
            "[tinyagents] attaching workspace descriptor"
        );
        ctx = ctx.with_workspace(descriptor);
    }
    // Assemble the run's store registry: the tool-result artifact index (when
    // present) and — behind the default-ON session dual-write flag — the
    // session KV store, so the harness carries a handle to the same
    // `{workspace}/tinyagents_store/kv` tree the live dual-write mirrors into
    // (issue #4249, 04.1). Both stores share one registry so neither clobbers
    // the other. Reads stay legacy until 04.2; this registration is additive
    // and best-effort (a workspace-resolve failure just skips it).
    let mut stores: Option<StoreRegistry> = None;
    if let Some(index) = tool_result_artifact_index {
        stores
            .get_or_insert_with(StoreRegistry::new)
            .register(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE, index);
    }
    // `session_kv_store` self-gates on the dual-write flag (config default ON +
    // env kill switch), returning `None` when disabled or unresolvable.
    if let Some(session_kv) = crate::openhuman::session_import::live::session_kv_store().await {
        stores.get_or_insert_with(StoreRegistry::new).register(
            crate::openhuman::session_import::live::TINYAGENTS_SESSION_KV_STORE,
            session_kv,
        );
        tracing::debug!(
            "[session-store] registered session kv store on RunContext.stores under '{}'",
            crate::openhuman::session_import::live::TINYAGENTS_SESSION_KV_STORE
        );
    }
    if let Some(stores) = stores {
        ctx = ctx.with_stores(stores);
    }

    let streaming = on_progress.is_some();
    // Retain a clone of the progress sink so the turn can emit a terminal
    // `TurnCompleted` after the run (the harness event stream the bridge mirrors
    // has no run-completed event). Parent turns only — a sub-agent turn reports
    // via its `Subagent*` events, not a top-level `TurnCompleted`.
    //
    // #4457 (defect C): suppressed entirely when `defer_turn_completed_to_caller`
    // is set — the caller (chat/session path) emits the single terminal
    // `TurnCompleted` itself, after its post-run wrap-up finishes streaming.
    let turn_completed_sink = (subagent_scope.is_none() && !defer_turn_completed_to_caller)
        .then(|| on_progress.clone())
        .flatten();
    // A sink is needed to mirror progress (bridge), to observe model-call
    // completions for the cap pauser, or to persist a durable event journal
    // (issue #4249, 05.1). The journal must attach even for an unobserved
    // (`on_progress = None`) turn so the run stays reconstructable, so the
    // EventSink is now created unconditionally — cheap (an empty sink) and, if
    // no consumer subscribes, inert.
    //
    // Mint the durable run id *before* the sink and seed the sink stream prefix
    // with it (`with_stream_id`), so every persisted observation's `event_id` is
    // the restart-stable `{run_id}-evt-{offset}` a late-attach replay
    // reconstructs the timeline from (05.1). The same id keys the journal + status.
    let journal_run_id = journal::mint_run_id();
    let events = Some(EventSink::with_stream_id(journal_run_id.as_str()));

    // Attach the event bridge for EVERY turn — including an unobserved
    // (`on_progress = None`) background/cron turn (#4467, item 3). The bridge's
    // `record_usage` feeds the global cost tracker on each `UsageRecorded` event
    // *during* the run, so a run that burns N model calls and then fails still
    // contributes that spend to the wallet/cost surfaces — the post-run
    // `record_unobserved_turn_usage` fallback below only runs on the success path
    // and never sees a failed run's usage. With `on_progress = None` the bridge
    // still records cost but its progress `send`s are inert no-ops, so there is
    // no spurious streaming. `events` is created unconditionally above, so the
    // bridge is always present.
    let bridge = events.as_ref().map(|events| {
        let bridge = OpenhumanEventBridge::with_scope(
            on_progress,
            model,
            provider_id.clone(),
            max_iterations,
            subagent_scope.clone(),
            cursor.clone(),
            tool_names.clone(),
            failure_map.clone(),
            provider_usage_carry.clone(),
        );
        events.subscribe(bridge.clone());
        bridge
    });

    // Cap pauser: stop gracefully at the model-call budget (returning the partial
    // transcript) so the caller can summarize a checkpoint instead of erroring.
    if pause_at_cap {
        if let (Some(events), Some(handle)) = (&events, &handle) {
            events.subscribe(CapPauser::new(handle.clone(), max_iterations));
        }
    }

    // Durable event journal + status store (issue #4249, 05.1). Attached *in
    // addition to* the bridge above: the EventSink fans out to both, so the
    // existing progress/global-bus path is untouched. Best-effort and non-fatal
    // — a failure to open/attach the journal returns `None` and the turn runs
    // unaffected. The handle stamps the terminal status once the run returns.
    // A sub-agent turn records under its task scope as the status thread id, so
    // `list_by_thread` can enumerate a task's runs (full parent/root lineage is
    // a 05.2/05.3 follow-up).
    let journal_thread_id = subagent_scope
        .as_ref()
        .map(|scope| tinyagents::harness::ids::ThreadId::new(scope.task_id.clone()));
    let turn_journal = match &events {
        Some(events) => {
            journal::attach_turn_journal(events, model, journal_run_id.clone(), journal_thread_id)
                .await
        }
        None => None,
    };
    if subagent_scope.is_none() {
        if let Some(crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
            request_id: Some(request_id),
            ..
        }) = crate::openhuman::agent::turn_origin::current()
        {
            journal::register_request_journal_run(&request_id, journal_run_id.as_str());
        }
    }

    if let Some(events) = &events {
        ctx = ctx.with_events(events.clone());
    }

    // Steering: attach the shared handle (when present), drain any already-queued
    // steer messages into it (so a pre-run steer lands before the first model
    // call), and forward mid-flight steers via a poll loop. The same handle
    // carries the early-exit `Pause`.
    //
    // Best-effort thread label for the delivery/requeue observability events and
    // the metadata on any requeued steer: a sub-agent uses its task id; the
    // interactive/channel parent turn reads the task-local turn origin.
    let steer_thread_label = subagent_scope
        .as_ref()
        .map(|s| s.task_id.clone())
        .or_else(|| match crate::openhuman::agent::turn_origin::current() {
            Some(crate::openhuman::agent::turn_origin::AgentTurnOrigin::WebChat {
                thread_id,
                ..
            }) => Some(thread_id),
            Some(crate::openhuman::agent::turn_origin::AgentTurnOrigin::ExternalChannel {
                reply_target,
                ..
            }) => Some(reply_target),
            _ => None,
        })
        .unwrap_or_default();

    // The forwarder is wrapped in an abort-on-drop RAII guard (issue #4456): its
    // `Drop` aborts the poll task, deregisters the sub-agent steering handle, and
    // drains residual (delivered-but-unapplied) steers back into the session run
    // queue. Because the guard is held across the drive future, that cleanup runs
    // identically on normal return, error, AND drop-cancellation — the previous
    // manual `forwarder.abort()` after the drive future only ran on normal
    // return, so a cancelled turn (web interrupt / sub-agent abort, both
    // drop-based) leaked a forwarder task that looped forever and raced the next
    // turn for the shared run queue.
    let steering_forwarder_guard = if let Some(handle) = handle {
        let registry_task_id = if let Some(scope) = &subagent_scope {
            let task_id = orchestration::TaskId::new(scope.task_id.clone());
            orchestration::shared_steering_registry().register(task_id.clone(), handle.clone());
            tracing::debug!(
                task_id = scope.task_id.as_str(),
                "[tinyagents] registered subagent steering handle"
            );
            Some(task_id)
        } else {
            None
        };
        // Pre-run drain so a steer/collect queued before the turn started lands
        // ahead of the first model call.
        if let Some(queue) = run_queue.clone() {
            steering_forwarder::forward_steers(&queue, &handle, &steer_thread_label).await;
            steering_forwarder::forward_collects(&queue, &handle, &steer_thread_label).await;
        }
        ctx = ctx.with_steering(handle.clone());
        Some(steering_forwarder::SteeringForwarderGuard::new(
            handle,
            run_queue,
            registry_task_id,
            steer_thread_label.clone(),
        ))
    } else {
        None
    };

    // Heap-allocate the harness drive future. It is large (it owns the whole run
    // context, middleware stack, and loop state), and a sub-agent turn runs
    // nested inside its parent's drive future — leaving it inline on the stack
    // overflows when the parent + child drives compose. Boxing keeps only a
    // pointer on the stack at each level.
    let run_result = with_run_cancellation(cancellation.clone(), async {
        if streaming {
            let mut stream = Box::pin(harness.invoke_stream_in_context(&(), ctx, input));
            let mut terminal = None;
            while let Some(item) = stream.next().await {
                match item {
                    AgentStreamItem::Event(_) => {}
                    AgentStreamItem::Completed(run) => {
                        terminal = Some(Ok(*run));
                        break;
                    }
                    AgentStreamItem::Failed(error) => {
                        terminal = Some(Err(tinyagents::TinyAgentsError::Model(error)));
                        break;
                    }
                }
            }
            terminal.unwrap_or_else(|| {
                Err(tinyagents::TinyAgentsError::Model(
                    "tinyagents stream ended without terminal run".to_string(),
                ))
            })
        } else {
            Box::pin(harness.invoke_in_context(&(), ctx, input)).await
        }
    })
    .await;
    // Drive future returned: run cleanup now (abort poll task + deregister +
    // requeue residual steers) rather than deferring to end-of-scope so the poll
    // loop cannot deliver into the no-longer-drained handle during post-run
    // journal/mapping work. On a *cancelled* turn this line is never reached; the
    // guard's `Drop` fires as the turn future unwinds, giving identical cleanup.
    drop(steering_forwarder_guard);
    let run = match run_result {
        Ok(run) => run,
        Err(e) => {
            // Durable journal: stamp the terminal failed status (best-effort,
            // non-fatal) before unwinding through the typed-error mapping below.
            if let Some(journal) = &turn_journal {
                journal.finish_failed(&e.to_string()).await;
            }
            // #4457 (defect B): map the run's *own* definitively-non-provider
            // failure kinds FIRST, before consulting `error_slot`. The slot
            // preserves the last provider error the model adapter saw — but the
            // adapter now clears it on every successful call (see
            // `ProviderModel::chat`/`stream`), so a stale slot should not exist
            // here. Ordering the cap/depth mappings ahead of the slot is
            // defense-in-depth: a run that failed on the model-call cap or a
            // spawn-depth limit is not a provider error, so it must surface as
            // `MaxIterationsExceeded` / the depth error rather than a leftover
            // provider error (wrong classification, wrong Sentry suppression,
            // wrong user message).
            //
            // The model-call cap (when not pausing gracefully — the channel/CLI
            // path) maps to the typed `AgentError::MaxIterationsExceeded` so
            // callers downcast it (Sentry skip) and render the canonical
            // "Agent exceeded maximum tool iterations" message, matching the
            // legacy `ErrorCheckpoint`.
            if let tinyagents::TinyAgentsError::LimitExceeded(msg) = &e {
                if msg.contains("model call") {
                    tracing::debug!(
                        model,
                        "[tinyagents] run hit the model-call cap; mapping to MaxIterationsExceeded (not consulting error_slot) — #4457 defect B"
                    );
                    return Err(anyhow::Error::new(
                        crate::openhuman::agent::error::AgentError::MaxIterationsExceeded {
                            max: max_iterations,
                        },
                    ));
                }
            }
            if let Some(depth_err) = tinyagents_depth_error(&e) {
                return Err(anyhow::Error::new(depth_err));
            }
            // Otherwise prefer the original typed provider error (preserves
            // `AgentError` downcasts the caller relies on) over the harness's
            // string wrap — this is where a genuine model/provider failure that
            // halted the run is re-surfaced with its real classification.
            // #4469 item 3: `into_inner` recovers a poisoned slot so a panic in
            // one run can't cascade into a second panic here that would mask the
            // original typed provider error.
            if let Some(original) = error_slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
            {
                tracing::debug!(
                    model,
                    "[tinyagents] re-surfacing typed provider error from error_slot as the run failure — #4457 defect B"
                );
                return Err(original);
            }
            return Err(anyhow::anyhow!("tinyagents harness run failed: {e}"));
        }
    };
    // Durable journal: the harness returned a transcript, so stamp the terminal
    // completed status (best-effort, non-fatal). The event stream carries no
    // run-terminal event, so this caller-driven write is authoritative.
    if let Some(journal) = &turn_journal {
        journal.finish_completed().await;
    }
    // Context-compression provenance (issue #4249, 03.1 item 6): the harness's
    // `AgentEvent::Compressed` projection only carries token deltas, so drain the
    // compression middleware's `records()` here — each carries the full
    // `CompressionProvenance` (source ids + before/after token estimates + policy
    // reason) built by `ProviderModelSummarizer`. Surfaced at info with a
    // grep-friendly `[context]` prefix so every compaction is auditable, not just
    // its net token saving.
    if let Some(mw) = &compression_mw {
        let records = mw.records();
        if !records.is_empty() {
            tracing::info!(
                model,
                compactions = records.len(),
                "[context] turn performed {} context compaction(s); surfacing provenance",
                records.len()
            );
            for (idx, record) in records.iter().enumerate() {
                let provenance = &record.provenance;
                tracing::info!(
                    model,
                    compaction = idx + 1,
                    of = records.len(),
                    source_count = provenance.source_ids.len(),
                    source_ids = ?provenance.source_ids,
                    from_tokens = provenance.original_token_estimate,
                    to_tokens = provenance.summary_token_estimate,
                    saved_tokens = provenance
                        .original_token_estimate
                        .saturating_sub(provenance.summary_token_estimate),
                    reason = %provenance.reason,
                    "[context] compaction provenance: folded {} source message(s) ({} -> {} tokens)",
                    provenance.source_ids.len(),
                    provenance.original_token_estimate,
                    provenance.summary_token_estimate,
                );
            }
        }
    }

    // Prompt-cache layout diagnostics (issue #4249, 03.2): drain the crate
    // `PromptCacheGuardMiddleware`'s recorded `CacheLayoutEvent`s and surface each
    // as a structured `[cache]` warning. Fires only when the cacheable prompt
    // prefix (system prompt + tool set) changed across model calls — i.e. volatile
    // content silently busting the provider KV-cache prefix. This is now the sole
    // owner of KV-cache-prefix drift detection: the warn-only
    // `CacheAlignMiddleware` was deleted in C3.
    let cache_layout_events = prompt_cache_guard.layout_events();
    if !cache_layout_events.is_empty() {
        tracing::debug!(
            model,
            events = cache_layout_events.len(),
            "[cache] surfacing prompt-cache layout change events"
        );
        observability::surface_cache_layout_events(model, &cache_layout_events);
    }

    // Terminal turn event (parity with the legacy engine's `progress::emit`): the
    // harness stream has no run-completed event, so emit `TurnCompleted` here with
    // the model-call count as the iteration total. Parent turns only; best-effort.
    // `turn_completed_sink` is `None` for sub-agent turns AND when the caller
    // opted to emit the terminal event itself after its post-run wrap-up
    // (`defer_turn_completed_to_caller`, #4457 defect C) — so this is the single
    // emission point for callers with no post-run streaming (channel/CLI).
    if let Some(sink) = &turn_completed_sink {
        let _ = sink.try_send(AgentProgress::TurnCompleted {
            iterations: run.model_calls as u32,
        });
    }

    // Response-cache effectiveness for this turn (issue #4249, 03.2). Additive —
    // logged with a grep-friendly `[cache]` prefix here; wiring the counts into the
    // cost-footer DTO is a follow-up coordinated with workstream 06. Only the
    // observed (bridge) path accumulates these; deterministic internal runs that
    // attach a `ResponseCache` are where non-zero counts appear.
    if let Some(bridge) = &bridge {
        let (cache_hits, cache_misses) = bridge.cache_counts();
        if cache_hits > 0 || cache_misses > 0 {
            tracing::debug!(
                model,
                cache_hits,
                cache_misses,
                "[cache] turn response-cache summary"
            );
        }
    }

    let bridge_totals = bridge.map(|bridge| bridge.totals_with_cost());

    // Prefer the bridge's accumulated usage (per-call, authoritative — including
    // cached tokens and the estimated charged USD) when the observed path ran;
    // otherwise fall back to the run's aggregate totals and estimate the cost from
    // them so a fire-and-forget turn still reports a real (non-$0) cost.
    let (input_tokens, output_tokens, cached_input_tokens, charged_amount_usd) = bridge_totals
        .unwrap_or_else(|| {
            let input = run.usage.usage.input_tokens;
            let output = run.usage.usage.output_tokens;
            let cached = run.usage.usage.cache_read_tokens;
            let charged =
                crate::openhuman::cost::catalog::estimate_cost_usd(model, input, output, cached);
            record_unobserved_turn_usage(model, input, output, cached, charged);
            (input, output, cached, charged)
        });

    // An early-exit tool fired: the loop paused after its round. Surface the tool
    // name and use its captured question as the turn text (the paused assistant
    // turn carries the tool call, not a final answer) so the caller can
    // checkpoint and prompt the user — matching the legacy `early_exit_tool`.
    let early_exit = early_exit_hook.and_then(|hook| hook.take());

    // Cap detection: the harness sets `final_response` only when the loop
    // finishes naturally (the model stopped requesting tools). When the cap
    // pauser stops the loop mid-work, `final_response` stays `None` — that's the
    // cap hit. An early-exit is a clean pause and takes precedence; under
    // `pause_at_cap` the only other `Pause` source is the cap pauser, so this is
    // unambiguous. (`run_queue` steering injects messages, never pauses.)
    // The repeated-failure breaker halts the run with a root-cause summary instead
    // of a final model turn; surface it as the turn's text so the no-progress cause
    // reaches the caller/user rather than an empty reply.
    let breaker_halt = halt_summary.lock().ok().and_then(|mut s| s.take());

    // Cap detection: the harness sets `final_response` only when the loop
    // finishes naturally (the model stopped requesting tools). When the cap
    // pauser stops the loop mid-work, `final_response` stays `None` — that's the
    // cap hit. An early-exit is a clean pause and takes precedence; under
    // `pause_at_cap` the only other `Pause` source is the cap pauser, so this is
    // unambiguous. (`run_queue` steering injects messages, never pauses.) A
    // breaker halt is *not* a cap hit: it already carries a root-cause summary, so
    // treating it as a cap would let the caller (sub-agent runner) overwrite that
    // summary with a generic checkpoint digest.
    let hit_cap = pause_at_cap
        && early_exit.is_none()
        && breaker_halt.is_none()
        && run.model_calls >= max_iterations
        && run.final_response.is_none();

    let (early_exit_tool, mut text) = match early_exit {
        Some(exit) => (Some(exit.tool), exit.question),
        None => (None, run.text().unwrap_or_default()),
    };

    // Carry the breaker halt onto the outcome so the sub-agent runner can report
    // `Incomplete` (#4466). `text` is overridden with the same root-cause summary
    // so callers with no breaker-awareness still surface the cause, not an empty
    // last-model reply.
    if let Some(summary) = &breaker_halt {
        tracing::info!(
            model,
            subagent = subagent_scope.is_some(),
            "[tinyagents] run halted by circuit breaker; surfacing as breaker_halt (#4466)"
        );
        text = summary.clone();
    }

    let tool_outcomes = tool_outcome_sink
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    let conversation = convert::messages_to_conversation(convert::messages_since_request(
        &run.messages,
        request_base_len,
    ));
    tracing::debug!(
        model,
        request_base_len,
        transcript_len = run.messages.len(),
        persisted_messages = run.messages.len().saturating_sub(request_base_len),
        subagent = subagent_scope.is_some(),
        "[tinyagents] persisting post-request transcript (shared path; steer-safe boundary)"
    );

    Ok(TinyagentsTurnOutcome {
        text,
        history: convert::messages_to_history(&run.messages),
        conversation,
        model_calls: run.model_calls,
        tool_calls: run.tool_calls,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        charged_amount_usd,
        early_exit_tool,
        hit_cap,
        breaker_halt,
        tool_outcomes,
    })
}

fn tinyagents_depth_error(
    err: &tinyagents::TinyAgentsError,
) -> Option<crate::openhuman::agent::harness::subagent_runner::SubagentRunError> {
    match err {
        tinyagents::TinyAgentsError::SubAgentDepth(max_depth)
        | tinyagents::TinyAgentsError::RecursionLimit(max_depth) => {
            Some(
                crate::openhuman::agent::harness::subagent_runner::SubagentRunError::SpawnDepthExceeded {
                    attempted_depth: max_depth.saturating_add(1),
                    max_depth: *max_depth,
                },
            )
        }
        _ => None,
    }
}

/// The per-turn crate [`ChatModel`](tinyagents::harness::model::ChatModel) set,
/// built once from an openhuman [`Provider`] by [`build_turn_models`] — the
/// single place a turn's `ProviderModel`s are constructed (issue #4249, Phase 5).
///
/// [`assemble_turn_harness`] takes this bundle instead of the raw provider, so
/// the harness assembly is expressed purely in crate model types; the
/// `Provider` → `ChatModel` adaptation is confined to `build_turn_models`.
pub(crate) struct TurnModels {
    /// The turn's effective/primary model (registry default + dispatch target).
    primary: Arc<dyn tinyagents::harness::model::ChatModel<()>>,
    /// Additive workload-tier routes (registry name → model), excluding the
    /// primary; the crate registry resolves fallback/selection across them.
    routes: Vec<(String, Arc<dyn tinyagents::harness::model::ChatModel<()>>)>,
    /// A model for the context-window summarizer (a distinct adapter instance so
    /// its provider errors don't touch the turn's `error_slot`).
    summarizer: Arc<dyn tinyagents::harness::model::ChatModel<()>>,
    /// Recovers the primary's original (downcastable) provider error on failure.
    error_slot: crate::openhuman::tinyagents::model::ProviderErrorSlot,
    /// Provider telemetry id (`{provider_id}.{model}` in Langfuse), captured from
    /// the source `Provider` at build time. Carried here (issue #4249, Phase 3 /
    /// Motion A) so the harness turn path no longer reads it off a raw
    /// `Provider` — the harness holds crate model types only.
    provider_id: String,
    /// The primary model's effective context window (drives the context-window
    /// summarization step). Resolved by the producer/factory before build so the
    /// harness graph no longer makes the async `effective_context_window` call.
    context_window: Option<u64>,
    /// Whether the source provider does native tool-calling — the harness uses
    /// this only to pick the history-suffix dispatcher (native envelope vs
    /// prompt-guided text). Captured from the provider at build time.
    native_tools: bool,
    /// Whether the source provider is vision-capable — the harness uses this to
    /// gate multimodal placeholder rehydration. Captured at build time.
    supports_vision: bool,
}

impl TurnModels {
    /// Provider telemetry id for this turn (`{provider_id}.{model}`).
    pub(crate) fn provider_id(&self) -> &str {
        &self.provider_id
    }

    /// The primary model's effective context window, if known.
    pub(crate) fn context_window(&self) -> Option<u64> {
        self.context_window
    }

    /// Whether the source provider does native tool-calling.
    pub(crate) fn native_tools(&self) -> bool {
        self.native_tools
    }

    /// Whether the source provider is vision-capable.
    pub(crate) fn supports_vision(&self) -> bool {
        self.supports_vision
    }
}

/// Build the per-turn [`TurnModels`] from an openhuman [`Provider`] — the sole
/// `ProviderModel` construction site for a turn (issue #4249, Phase 5). The
/// primary carries the model's context window on its capability profile; the
/// workload-tier routes are projected via [`routes::build_route_models`]; the
/// summarizer is a separate adapter over the same provider/model.
pub(crate) fn build_turn_models(
    provider: Arc<dyn Provider>,
    model: &str,
    temperature: f64,
    context_window: Option<u64>,
) -> TurnModels {
    // Capture the provider metadata the harness turn path used to read directly
    // off the raw `Provider` (issue #4249, Phase 3 / Motion A). Recording it on
    // `TurnModels` is what lets `AgentTurnRequest`/`Agent`/the harness graph hold
    // crate model types only — no `dyn Provider` above the seam.
    let provider_id = provider.telemetry_provider_id();
    let native_tools = provider.supports_native_tools();
    let supports_vision = provider.supports_vision();
    let summary_provider = provider.clone();
    let mut primary = ProviderModel::new(provider, model, temperature);
    // Record the model's context window on its capability profile (issue #4249,
    // Phase 2) so the crate can validate input capacity before dispatch. The
    // per-call output cap rides `RunConfig.max_turn_output_tokens` instead.
    if let Some(window) = context_window.filter(|w| *w > 0) {
        primary = primary.with_context_window(window);
    }
    let error_slot = primary.error_slot();
    let primary: Arc<dyn tinyagents::harness::model::ChatModel<()>> = Arc::new(primary);

    let routes = routes::build_route_models(&summary_provider, temperature, model)
        .into_iter()
        .map(|route| {
            let model: Arc<dyn tinyagents::harness::model::ChatModel<()>> = route.model;
            (route.name, model)
        })
        .collect();

    // A distinct adapter instance for the summarizer (own error_slot), matching
    // the pre-Phase-5 separate `summary_provider` clone.
    let summarizer: Arc<dyn tinyagents::harness::model::ChatModel<()>> =
        Arc::new(ProviderModel::new(summary_provider, model, temperature));

    TurnModels {
        primary,
        routes,
        summarizer,
        error_slot,
        provider_id,
        context_window,
        native_tools,
        supports_vision,
    }
}

/// Build the per-turn [`TurnModels`] **crate-natively** from `(role, config)` —
/// the Phase 3 P3-B cutover of [`build_turn_models`]: instead of wrapping one host
/// `Provider` per tier in a [`ProviderModel`], each tier is built as a crate-native
/// [`ChatModel`] via [`factory::create_turn_chat_model`] (managed →
/// `OpenHumanBackendModel`, local/cloud → crate `OpenAiModel`).
///
/// The `TurnModels` shape is identical to [`build_turn_models`] so
/// [`assemble_turn_harness`] is unchanged. The provider metadata
/// (`provider_id` / `native_tools` / `supports_vision`) is derived by the caller
/// ([`TurnModelSource::build`]) from the resolved provider string and config.
/// `error_slot` is a fresh
/// empty slot — crate-native models surface `TinyAgentsError` directly (no
/// downcastable `anyhow` to preserve), so typed provider-error *recovery* is unused
/// here (Sentry suppression is unaffected — both `skips_sentry` cases are raised in
/// the host turn loop).
#[allow(clippy::too_many_arguments)]
fn build_turn_models_crate(
    role: &str,
    config: &crate::openhuman::config::Config,
    model: &str,
    temperature: f64,
    context_window: Option<u64>,
    primary_override: Option<&str>,
    provider_id: String,
    native_tools: bool,
    supports_vision: bool,
    force_text_mode: bool,
) -> anyhow::Result<TurnModels> {
    use crate::openhuman::inference::provider::factory;

    // The primary honours an explicit provider-string override when the producer's
    // effective provider differs from `provider_for_role(role)` (triage #1257).
    let build_primary =
        |m: &str| -> anyhow::Result<Arc<dyn tinyagents::harness::model::ChatModel<()>>> {
            match primary_override {
                Some(ps) => factory::create_turn_chat_model_from_string_with_native_tools(
                    role,
                    ps,
                    config,
                    m,
                    temperature,
                    !force_text_mode,
                ),
                None => factory::create_turn_chat_model_with_native_tools(
                    role,
                    config,
                    m,
                    temperature,
                    !force_text_mode,
                ),
            }
        };

    let primary = build_primary(model)?;

    // Additive workload-tier routes: one crate-native model per tier (skipping the
    // turn's own model, which is registered as the default primary), each pinned to
    // the tier alias so the crate registry resolves cross-route fallback across them.
    let mut routes: Vec<(String, Arc<dyn tinyagents::harness::model::ChatModel<()>>)> = Vec::new();
    for &tier in routes::WORKLOAD_ROUTE_TIERS {
        if tier == model {
            continue;
        }
        let tier_role = factory::role_for_model_tier(tier);
        match factory::create_turn_chat_model(tier_role, config, tier, temperature) {
            Ok(route_model) => routes.push((tier.to_string(), route_model)),
            Err(e) => {
                // A route that can't be built (e.g. an unconfigured BYOK tier) is
                // skipped, not fatal — the primary still dispatches (parity with the
                // `Provider` path, where an unresolved tier simply isn't registered).
                tracing::debug!(
                    route = tier,
                    error = %e,
                    "[models] skipping crate-native workload route that failed to build"
                );
            }
        }
    }

    // The summarizer is a distinct adapter instance (own empty error slot).
    let summarizer = build_primary(model)?;

    Ok(TurnModels {
        primary,
        routes,
        summarizer,
        error_slot: Arc::new(std::sync::Mutex::new(None)),
        provider_id,
        context_window,
        native_tools,
        supports_vision,
    })
}

/// A model-agnostic source of per-turn [`TurnModels`] — the seam-owned handle the
/// agent harness holds instead of a raw `Arc<dyn Provider>` (issue #4249, Phase 3
/// / Motion A).
///
/// An [`Agent`](crate::openhuman::agent::Agent) (and each channel/subagent turn
/// request) is model-agnostic: it holds this source and builds a *tiered* crate
/// [`ChatModel`] set (primary + workload-tier fallback routes + summarizer) per
/// turn. Production sources retain only crate-native role/config metadata;
/// provider-backed sources remain for injected tests and bespoke clients. Constructed in
/// exactly one place — [`create_turn_model_source`](crate::openhuman::inference::provider::factory::create_turn_model_source).
#[derive(Clone)]
pub struct TurnModelSource {
    provider: Option<Arc<dyn Provider>>,
    /// When set, [`build`](Self::build) / [`build_summarizer`](Self::build_summarizer)
    /// construct **crate-native** models from `(role, config)` (Phase 3 P3-B) via
    /// [`build_turn_models_crate`]. Crate-native sources keep `provider` as
    /// `None`; build failures propagate instead of falling back to the host wire
    /// client.
    crate_native: Option<CrateNativeSource>,
    force_text_mode: bool,
}

/// The `(role, config)` a crate-native [`TurnModelSource`] builds its tiered
/// [`TurnModels`] from per turn.
#[derive(Clone)]
struct CrateNativeSource {
    role: String,
    config: Arc<crate::openhuman::config::Config>,
    /// An explicit provider string for the **primary** model, overriding the
    /// role's default resolution. Set when a producer's effective provider differs
    /// from `provider_for_role(role)` — e.g. triage's #1257 force-managed override
    /// (`build_remote_provider`). `None` builds the primary from `role`. Routes
    /// always use the standard workload tiers.
    primary_override: Option<String>,
    force_text_mode: bool,
}

impl TurnModelSource {
    /// Wrap a resolved provider. The host↔seam boundary: the inference factory
    /// (or a producer holding a resolved provider) builds a source here; the
    /// agent harness above the seam then names only `TurnModelSource`, never the
    /// `Provider` trait. `pub` so the native-bus request type
    /// ([`AgentTurnRequest`](crate::openhuman::agent::bus::AgentTurnRequest)) and
    /// its integration tests can construct one.
    pub fn new(provider: Arc<dyn Provider>) -> Self {
        Self {
            provider: Some(provider),
            crate_native: None,
            force_text_mode: false,
        }
    }

    /// Build a crate-native source: [`build`](Self::build) constructs the tiered
    /// [`TurnModels`] from `(role, config)` via [`build_turn_models_crate`] rather
    /// than wrapping a provider in `ProviderModel`s. Used by the session-builder producer
    /// (`crate_native_provider`); the triage path uses
    /// [`new_crate_native_from_string`](Self::new_crate_native_from_string).
    pub(crate) fn new_crate_native(
        role: impl Into<String>,
        config: Arc<crate::openhuman::config::Config>,
    ) -> Self {
        Self {
            provider: None,
            crate_native: Some(CrateNativeSource {
                role: role.into(),
                config,
                primary_override: None,
                force_text_mode: false,
            }),
            force_text_mode: false,
        }
    }

    /// Build a crate-native source whose **primary** model is built from an explicit
    /// `provider_string` (via [`factory::create_turn_chat_model_from_string`]) rather
    /// than the role's default resolution — the triage path's #1257 force-managed
    /// override (`build_remote_provider` picks the effective string). Routes still
    /// use the standard workload tiers.
    pub(crate) fn new_crate_native_from_string(
        role: impl Into<String>,
        provider_string: impl Into<String>,
        config: Arc<crate::openhuman::config::Config>,
    ) -> Self {
        Self {
            provider: None,
            crate_native: Some(CrateNativeSource {
                role: role.into(),
                config,
                primary_override: Some(provider_string.into()),
                force_text_mode: false,
            }),
            force_text_mode: false,
        }
    }

    /// Force prompt-guided tool calling without resolving a host provider.
    pub(crate) fn with_text_mode(mut self) -> Self {
        self.force_text_mode = true;
        if let Some(source) = self.crate_native.as_mut() {
            source.force_text_mode = true;
        }
        self
    }

    /// Resolve the model's effective context window (async provider probe) — the
    /// value that drives the context-window summarization step. Resolved before
    /// [`build`](Self::build) so the harness graph makes no async `Provider` call.
    pub(crate) async fn effective_context_window(&self, model: &str) -> Option<u64> {
        if let Some(provider) = &self.provider {
            return provider.effective_context_window(model).await;
        }
        let provider_string = self.crate_native.as_ref().map(|source| {
            source.primary_override.clone().unwrap_or_else(|| {
                crate::openhuman::inference::provider::provider_for_role(
                    &source.role,
                    &source.config,
                )
            })
        });
        let local_kind = provider_string
            .as_deref()
            .and_then(crate::openhuman::inference::local::profile::kind_from_provider_string);
        crate::openhuman::inference::model_context::context_window_for_model_with_local_fallback(
            model, local_kind,
        )
    }

    /// Whether the underlying provider is a local runtime (Ollama / LM Studio).
    /// A passthrough so callers (e.g. the sub-agent summarization-route decision)
    /// can branch on locality without naming the `Provider` trait.
    pub(crate) fn is_local_provider(&self) -> bool {
        if let Some(provider) = &self.provider {
            return provider.is_local_provider();
        }
        self.crate_native.as_ref().is_some_and(|source| {
            let provider = source.primary_override.clone().unwrap_or_else(|| {
                crate::openhuman::inference::provider::provider_for_role(
                    &source.role,
                    &source.config,
                )
            });
            crate::openhuman::inference::local::profile::is_local_provider_string(&provider)
        })
    }

    /// The underlying provider handle. An escape hatch for the few seam-boundary
    /// sites that still resolve/inherit a raw provider (sub-agent provider
    /// resolution + its unit tests, the rhai-workflow model build): they consume
    /// it inline rather than holding it, so no agent-harness *struct* carries an
    /// `Arc<dyn Provider>`. Shrinks further as those callers move to the crate
    /// `ModelRegistry` (Motion B).
    pub(crate) fn provider(&self) -> anyhow::Result<Arc<dyn Provider>> {
        if let Some(provider) = &self.provider {
            return Ok(provider.clone());
        }
        let source = self
            .crate_native
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("turn model source has no provider configuration"))?;
        let built = match source.primary_override.as_deref() {
            Some(provider) => {
                crate::openhuman::inference::provider::factory::create_chat_provider_from_string(
                    &source.role,
                    provider,
                    &source.config,
                )
            }
            None => crate::openhuman::inference::provider::create_chat_provider(
                &source.role,
                &source.config,
            ),
        }?;
        Ok(Arc::from(built.0))
    }

    /// Build this turn's [`TurnModels`] (primary + tier routes + summarizer),
    /// capturing provider telemetry id + capabilities onto the bundle.
    pub(crate) fn build(
        &self,
        model: &str,
        temperature: f64,
        context_window: Option<u64>,
    ) -> anyhow::Result<TurnModels> {
        if let Some(cn) = &self.crate_native {
            let provider_string = cn.primary_override.clone().unwrap_or_else(|| {
                crate::openhuman::inference::provider::provider_for_role(&cn.role, &cn.config)
            });
            let is_local = crate::openhuman::inference::local::profile::is_local_provider_string(
                &provider_string,
            );
            let provider_id = if provider_string == "openhuman"
                || provider_string.is_empty()
                || provider_string == "cloud"
            {
                "managed".to_string()
            } else {
                provider_string
                    .split(':')
                    .next()
                    .unwrap_or(&provider_string)
                    .to_string()
            };
            return build_turn_models_crate(
                &cn.role,
                &cn.config,
                model,
                temperature,
                context_window,
                cn.primary_override.as_deref(),
                provider_id,
                !is_local,
                !is_local,
                cn.force_text_mode,
            );
        }
        let provider = self.provider.as_ref().ok_or_else(|| {
            anyhow::anyhow!("provider-backed turn source is missing its provider")
        })?;
        let mut models = build_turn_models(provider.clone(), model, temperature, context_window);
        if self.force_text_mode {
            let primary =
                ProviderModel::new(provider.clone(), model, temperature).with_tool_calling(false);
            let primary = if let Some(window) = context_window.filter(|w| *w > 0) {
                primary.with_context_window(window)
            } else {
                primary
            };
            models.error_slot = primary.error_slot();
            models.primary = Arc::new(primary);
            models.native_tools = false;
        }
        Ok(models)
    }

    /// Build a standalone summarizer [`ChatModel`](tinyagents::harness::model::ChatModel)
    /// over this source's provider — a fresh adapter (own error slot) for one-off
    /// summary calls outside the main turn (e.g. the sub-agent cap-hit checkpoint),
    /// so the caller can `invoke` without naming the `Provider` trait. The output
    /// cap rides the per-call `ModelRequest`, not the model.
    pub(crate) fn build_summarizer(
        &self,
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<Arc<dyn tinyagents::harness::model::ChatModel<()>>> {
        if let Some(cn) = &self.crate_native {
            let built = match cn.primary_override.as_deref() {
                Some(ps) => crate::openhuman::inference::provider::factory::create_turn_chat_model_from_string(
                    &cn.role, ps, &cn.config, model, temperature,
                ),
                None => crate::openhuman::inference::provider::factory::create_turn_chat_model(
                    &cn.role,
                    &cn.config,
                    model,
                    temperature,
                ),
            };
            return built;
        }
        let provider = self.provider.as_ref().ok_or_else(|| {
            anyhow::anyhow!("provider-backed turn source is missing its provider")
        })?;
        Ok(Arc::new(ProviderModel::new(
            provider.clone(),
            model,
            temperature,
        )))
    }
}

/// Everything [`assemble_turn_harness`] wires up for one turn: the configured
/// harness plus the shared slots/handles the run loop reads after the drive
/// future returns.
struct AssembledTurnHarness {
    /// The fully assembled harness: model, tools, and middleware registered in
    /// the intended order.
    harness: AgentHarness<()>,
    /// Shared 1-based model-call cursor (event bridge advances, model adapter
    /// reads for out-of-band thinking attribution).
    cursor: IterationCursor,
    /// Shared `call_id → tool_name` map: the model adapter's `ThinkingForwarder`
    /// writes it on tool-call start; the event bridge reads it to label the
    /// tool-argument fragments it now projects off the crate stream.
    tool_names: ToolNameMap,
    /// Shared `call_id → (success, failure, elapsed_ms, output_chars)`
    /// side-channel: the tool-outcome capture middleware classifies each outcome
    /// + records its duration/output size; the event bridge reads it to project
    /// real success + a user-facing failure + timing onto `ToolCallCompleted`.
    failure_map: ToolFailureMap,
    /// Shared FIFO carry of per-call provider `UsageInfo` (charged USD + context
    /// window): the model adapter pushes, the event bridge pops when recording
    /// usage — restores charged-USD precedence on the tinyagents path (#4467).
    provider_usage_carry: ProviderUsageCarry,
    /// Recovers the original (downcastable) provider error on run failure.
    error_slot: crate::openhuman::tinyagents::model::ProviderErrorSlot,
    /// Root-cause summary recorded by the repeated-tool-failure breaker.
    halt_summary: HaltSummarySlot,
    /// Per-call tool success/content capture for honest `ToolCallRecord`s.
    tool_outcome_sink: ToolOutcomeSink,
    /// The shared steering handle (mid-flight steer, early-exit, cap, stop-hook
    /// pauses).
    handle: Option<SteeringHandle>,
    /// Records the first early-exit tool round, when early-exit tools exist.
    early_exit_hook: Option<EarlyExitHook>,
    /// Number of callable tools registered.
    tool_count: usize,
    /// TinyAgents named-capability projection for this turn. The live run still
    /// uses the harness registries above; this snapshot makes the projected
    /// model/tool/graph inventory inspectable without changing dispatch.
    registry_snapshot: RegistrySnapshot,
    /// Health diagnostics from the projected registry.
    registry_diagnostics: Vec<RegistryDiagnostic>,
    /// TinyAgents store index for OpenHuman action-dir tool-result artifacts.
    tool_result_artifact_index: Option<Arc<ToolResultArtifactIndexStore>>,
    /// Concrete handle to the installed [`ContextCompressionMiddleware`], when the
    /// summarization step is active. Drained after the run to surface each
    /// compaction's [`CompressionProvenance`][tinyagents::harness::summarization::CompressionProvenance]
    /// (source ids + before/after token estimates) via the observability path.
    compression_mw: Option<Arc<ContextCompressionMiddleware>>,
    /// Crate prompt-cache guard (issue #4249, 03.2). Records a `CacheLayoutEvent`
    /// whenever the cacheable prompt prefix (system prompt + tool set) changes
    /// across model calls. Drained after the run and surfaced via
    /// [`observability::surface_cache_layout_events`] — the crate-native
    /// replacement for the deleted `CacheAlignMiddleware` warn-log (C3).
    prompt_cache_guard: Arc<PromptCacheGuardMiddleware>,
}

/// Assemble the turn harness for [`run_turn_via_tinyagents_shared`]: register
/// the provider model, every shared tool, and the full middleware stack in the
/// intended order. Split out of the runner so the adapter inventory is directly
/// testable (issue #4249, Phase 11) — the returned [`AssembledTurnHarness`]
/// exposes the harness registries without driving a run.
#[allow(clippy::too_many_arguments)]
fn assemble_turn_harness(
    turn_models: TurnModels,
    model: &str,
    tool_sets: Vec<Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>>,
    allowed: Option<HashSet<String>>,
    max_iterations: usize,
    _on_progress: Option<Sender<AgentProgress>>,
    subagent_scope: Option<SubagentScope>,
    context_window: Option<u64>,
    early_exit_tools: &[&str],
    context_mw: TurnContextMiddleware,
    tool_policy: Option<ToolPolicyEnforcement>,
    required_capabilities: Option<CapabilitySet>,
    deterministic_cacheable: bool,
) -> AssembledTurnHarness {
    let mut harness: AgentHarness<()> = AgentHarness::new();
    // Cross-route fallback ownership (issue #4249, Workstream 02.2): populate the
    // SDK `RunPolicy.fallback` with the ordered same-family route chain for this
    // turn's primary model so the harness fails over to a sibling workload tier
    // (e.g. chat-v1 → burst-v1) when the primary route errors. Retry stays pinned
    // to a single attempt (see `run_policy_for`) — fallback and retry are
    // independent knobs, and only fallback is enabled here because `ReliableProvider`
    // (still wrapped) does not fail over across the registered tier routes.
    let mut policy = run_policy_for(max_iterations, deterministic_cacheable);
    let route_fallback = routes::route_fallback_policy(model);
    policy.fallback = route_fallback.clone();
    tracing::debug!(
        model,
        fallback_chain = ?route_fallback.as_ref().map(|f| &f.models),
        "[models] assembling turn harness with SDK retry/fallback policy"
    );
    harness.with_policy(policy);
    // Deterministic internal runs (summarizer/triage/memory-scoring style) may
    // reuse a prior identical model response; attach an in-memory response cache
    // so the agent loop can short-circuit a recurring provider call and emit
    // `CacheHit`/`CacheMiss` (issue #4249, 03.2). NEVER attached for interactive
    // chat turns — a live user turn must never be served a cached response.
    if deterministic_cacheable {
        harness.with_response_cache(Arc::new(InMemoryResponseCache::new()));
        tracing::debug!(
            model,
            "[cache] response cache attached (deterministic internal run)"
        );
    }
    let mut capability_registry: CapabilityRegistry<()> = CapabilityRegistry::new();

    let cursor: IterationCursor = Arc::default();
    // Shared `call_id → tool_name` map: the forwarder records the name on
    // tool-call start (the crate `ToolDelta` carries none), the bridge reads it
    // to label the argument fragments now streamed via `MessageDelta.tool_call`.
    let tool_names: ToolNameMap = Arc::default();
    // Shared FIFO carry of per-call provider `UsageInfo`: `UsageCarryMiddleware`
    // pushes each response's usage (charged USD + context window +
    // cache-creation/reasoning tokens, read off the response via G1), the event
    // bridge pops it when recording that call's usage (#4467, item 1). The carry
    // is produced by a wrap-model middleware now, not the adapter, so route models
    // carry no usage side-channel (Phase 5).
    let provider_usage_carry: ProviderUsageCarry = Arc::default();
    // The turn's models are pre-built by `build_turn_models` (the single
    // `ProviderModel` construction site) and handed in as crate `ChatModel`s —
    // the assembly no longer touches the raw provider (issue #4249, Phase 5).
    let TurnModels {
        primary,
        routes,
        summarizer: summarizer_model,
        error_slot,
        // Provider metadata (id/context-window/caps) is consumed by the turn-path
        // caller before dispatch, not by harness assembly.
        ..
    } = turn_models;
    capability_registry.replace_model(model, primary.clone());
    harness
        .register_model(model, primary)
        .set_default_model(model);

    // Project the full workload-route set into the registry (issue #4249,
    // Workstream 02.1). Each route is an additive registry entry carrying its
    // per-route capability profile; `set_default_model` above keeps the turn's
    // effective model as the dispatch target, so behavior is preserved until
    // fallback/selection (02.2) chooses among the routes. `build_turn_models`
    // already skipped the turn's own model, so we don't shadow the default.
    for (name, route_model) in routes {
        capability_registry.replace_model(name.as_str(), route_model.clone());
        harness.register_model(name, route_model);
    }

    // Cost usage capture (issue #4249, Phase 5): feed the event bridge's usage
    // carry from a wrap-model middleware that reads the full `UsageInfo` off each
    // response, instead of every `ProviderModel` pushing it. Installed
    // unconditionally — usage flows on every turn — and shares the same carry the
    // bridge drains on `UsageRecorded`.
    harness.push_model_middleware(Arc::new(routes::UsageCarryMiddleware::new(
        provider_usage_carry.clone(),
    )));

    // Per-call capability gate (issue #4249, Workstream 02.1): when the turn has
    // derivable capability needs (today: vision for a `vision-v1` turn), stamp
    // them onto every `ModelRequest` via `with_required_capabilities` so an unfit
    // model is rejected pre-dispatch (and, once 02.2 lands, a capable fallback is
    // selected) instead of failing at the provider.
    if let Some(required) = required_capabilities {
        harness.push_model_middleware(Arc::new(routes::RequiredCapabilitiesMiddleware::new(
            required,
        )));
    }

    // Fallback event parity (issue #4249, Workstream 02.2): the crate's
    // registry-backed `RunPolicy.fallback` traversal (wired above) performs the
    // cross-route swap silently — it emits no `AgentEvent::FallbackSelected`. Wrap
    // the model-resolving core with an observer that surfaces the parity event
    // whenever the resolved model differs from the primary, so a fallback is
    // visible on the OpenHuman progress/observability bridge (and grep-logged under
    // `[fallback]`). Installed only when a fallback chain exists; it never re-issues
    // the call, so it adds no provider dispatch (no double-fallback).
    if route_fallback.is_some() {
        harness.push_model_middleware(Arc::new(routes::FallbackObserverMiddleware::new(model)));
    }

    // Capture context settings before `install` consumes `context_mw`.
    let autocompact_enabled = context_mw.autocompact_enabled;
    let tool_result_artifact_index = context_mw
        .artifact_store
        .as_ref()
        .map(|_| Arc::new(ToolResultArtifactIndexStore::new()));

    // Snapshot the installed stop hooks while the `CURRENT_STOP_HOOKS`
    // task-local is in scope (the harness drive future runs inline on this
    // task, but capturing here keeps the wiring robust). When present they fire
    // via `StopHookMiddleware` and pause through the shared steering handle.
    let stop_hooks = crate::openhuman::agent::stop_hooks::current_stop_hooks();

    // A single steering handle drives mid-flight steering (run queue), the
    // early-exit pause, the model-call-cap pause, and stop-hook pauses, so they
    // all reach the same loop. Created when any of them is active.
    // A steering handle is always created now: besides run-queue steering, the
    // early-exit / cap / stop-hook pauses, the repeated-tool-failure breaker
    // (below) also pauses through it, and it wants to fire on every path
    // (including plain channel turns that set none of the other flags). An idle
    // handle is a no-op — the loop just drains an empty steering channel.
    // Tighten the steering allowlist by run class: a live interactive chat turn
    // keeps the InjectMessage/Pause allowlist exactly, while a detached
    // sub-agent run (identified by its `subagent_scope`) additionally accepts
    // graceful control-flow steering (Resume/Cancel/Redirect). `subagent_scope`
    // is the only run-class signal available at this steer site.
    let steering_run_class = if subagent_scope.is_some() {
        orchestration::SteeringRunClass::Background
    } else {
        orchestration::SteeringRunClass::Interactive
    };
    let handle = Some(orchestration::openhuman_steering_handle(steering_run_class));

    // Memory protocol (issue #4116): observe the read → dedupe → write →
    // update-index cycle and append a corrective note when a write skips the
    // dedupe read or leaves the index stale. Pushed first / outermost so its
    // `after_tool` runs *after* the byte-cap truncation, keeping the note.
    harness.push_middleware(Arc::new(middleware::MemoryProtocolMiddleware::new()));

    // Repeated-failure circuit breaker: pause the run when a tool returns the same
    // error `REPEATED_TOOL_FAILURE_THRESHOLD` times in a row, so a deterministic
    // security/approval denial or terminal tool error surfaces its root cause
    // instead of burning the whole iteration budget (legacy ProgressGuard parity).
    let halt_summary: HaltSummarySlot = std::sync::Arc::new(std::sync::Mutex::new(None));
    if let Some(handle) = &handle {
        harness.push_middleware(Arc::new(middleware::RepeatedToolFailureMiddleware::new(
            handle.clone(),
            REPEATED_TOOL_FAILURE_THRESHOLD,
            halt_summary.clone(),
        )));
    }

    // Repeat-progress breaker (issue #4463, restoring #4088 / #4095): the failure
    // breaker above resets on every success, so a model looping on a *successful*
    // no-op tool or re-emitting an identical narration+call never trips it. This
    // guard halts on identical successful `(tool, args)` batches / identical
    // outputs, sharing the same halt-summary slot + steering handle. Polling tools
    // (`wait_subagent`) stay exempt.
    if let Some(handle) = &handle {
        harness.push_middleware(Arc::new(middleware::RepeatProgressMiddleware::new(
            handle.clone(),
            halt_summary.clone(),
        )));
    }

    // Policy-driven stop hooks (budget cap, thread-goal budget, ad-hoc iteration
    // ceiling): fire after each model call and pause the run on the first stop
    // vote. Replaces the legacy tool-call-loop firing point.
    if let Some(handle) = &handle {
        if !stop_hooks.is_empty() {
            harness.push_middleware(Arc::new(stop_hooks::StopHookMiddleware::new(
                handle.clone(),
                model,
                max_iterations,
                stop_hooks,
            )));
        }
    }
    let early_exit_set: HashSet<&str> = early_exit_tools.iter().copied().collect();
    // One hook per run, shared by every early-exit adapter (records the first
    // early-exit and pauses). Requires the steering handle.
    let early_exit_hook = handle
        .as_ref()
        .filter(|_| !early_exit_set.is_empty())
        .map(|h| EarlyExitHook::new(h.clone()));

    // Register one adapter per unique callable tool name found across the shared
    // sets (newest set wins on a name clash). Allowlist semantics are
    // **fail-closed** (issue #4452): `allowed == None` → no filter, every visible
    // tool registers; `allowed == Some(set)` → register *exactly* the named
    // tools, so `Some(empty)` denies all. This is what keeps a deliberately
    // tool-less sub-agent (`ToolScope::Named([])`, a zero-match `skill_filter`,
    // or a `named` list that resolves to nothing) from silently inheriting the
    // parent's full tool surface (shell/file-write/spawn) — the old
    // `allowed.is_empty() || allowed.contains(name)` predicate was fail-open.
    let is_subagent_run = subagent_scope.is_some();
    if let Some(set) = &allowed {
        if set.is_empty() {
            tracing::warn!(
                subagent = is_subagent_run,
                "[subagent] tool allowlist resolved empty — registering no tools"
            );
        }
    }
    let mut seen_candidates: HashSet<String> = HashSet::new();
    let candidate_names: Vec<String> = tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .map(|tool| tool.name())
        .filter(|&name| seen_candidates.insert(name.to_string()))
        .map(|name| name.to_string())
        .collect();
    let mut registered: HashSet<String> = HashSet::new();
    for name in candidate_names.iter().map(String::as_str) {
        // Fail-closed allowlist: `None` admits everything, `Some(set)` admits only
        // its members (empty set → nothing).
        let admitted = match &allowed {
            None => true,
            Some(set) => set.contains(name),
        };
        // Defense-in-depth (issue #4452): a sub-agent must NEVER be handed a
        // spawn/delegate tool, regardless of what the resolved allowlist contains.
        // Re-assert the invariant here at registration time (not just on the
        // caller's `allowed_indices`) so a misbuilt allowlist can't reintroduce
        // `spawn_subagent`/`delegate_*`/worker-thread spawning into a child run.
        let spawn_stripped = is_subagent_run && is_subagent_spawn_or_delegate_tool(name);
        if spawn_stripped {
            tracing::warn!(
                tool = name,
                "[subagent] refusing to register spawn/delegate tool on sub-agent run"
            );
        }
        if !registered.contains(name) && admitted && !spawn_stripped {
            if let Some(mut adapter) = SharedToolAdapter::for_name(tool_sets.clone(), name) {
                if early_exit_set.contains(name) {
                    if let Some(hook) = &early_exit_hook {
                        adapter = adapter.with_early_exit(hook.clone());
                    }
                }
                registered.insert(name.to_string());
                let adapter = Arc::new(adapter);
                capability_registry.replace_tool(adapter.clone());
                harness.register_tool(adapter);
            }
        }
    }
    let tool_count = registered.len();
    for report in all_graph_topologies() {
        let _ = capability_registry.register_descriptor(ComponentKind::Graph, report.name);
    }

    // Project the agents visible to this turn into the registry as name-only
    // `ComponentKind::Agent` descriptors (issue #4249, Workstream 10.1). This is
    // metadata only: no executable `HarnessAgent` is attached (sub-agent
    // dispatch still flows through the openhuman sub-agent runner), so the
    // registration is cheap and leaves the turn hot path unchanged. Agents are
    // sourced from BOTH the runtime `AgentDefinitionRegistry` global (built-ins
    // plus any workspace/config custom overrides, already merged by id) AND the
    // `agent_registry` built-in loader, deduped by id — the runtime registry is
    // registered first and wins, since it carries the richer, override-aware
    // `when_to_use`. Registering here keeps the ids in
    // `capability_registry.snapshot()` so the 10.3 `agent.registry_snapshot` RPC
    // and the fail-closed diagnostics (10.2) observe them.
    //
    // DEFERRED (rich metadata): tinyagents 1.3.0 exposes no public API to attach
    // a `ComponentMetadata` description/tags to a name-only descriptor — only
    // `register_agent(Arc<dyn HarnessAgent>)` carries a full executable blueprint
    // we do not have at this layer. Until the crate grows a
    // `register_descriptor_with_meta` (or we thread real `HarnessAgent`s through
    // `assemble_turn_harness`), the `when_to_use` descriptions and
    // `display_name`/`source` tags cannot be persisted onto the snapshot entry;
    // only the ids are projected. The runtime-registry → executable-agent
    // projection (so `register_agent`/`.rag` sub-agent resolution can bind these)
    // remains the deferred follow-up.
    let mut registered_agents: HashSet<String> = HashSet::new();
    let mut runtime_agent_count = 0usize;
    if let Some(runtime) =
        crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
    {
        for def in runtime.list() {
            if registered_agents.insert(def.id.clone()) {
                let _ =
                    capability_registry.register_descriptor(ComponentKind::Agent, def.id.clone());
                runtime_agent_count += 1;
            }
        }
    }
    // agent_registry built-ins as a supplement/fallback (deduped by id). When the
    // runtime global is uninitialised this is the sole source; otherwise it only
    // contributes ids the runtime registry did not already cover.
    let mut builtin_supplement_count = 0usize;
    match crate::openhuman::agent_registry::agents::load_builtins() {
        Ok(builtins) => {
            for def in builtins {
                if registered_agents.insert(def.id.clone()) {
                    let _ = capability_registry
                        .register_descriptor(ComponentKind::Agent, def.id.clone());
                    builtin_supplement_count += 1;
                }
            }
        }
        Err(err) => {
            tracing::debug!(
                %err,
                "[registry] agent_registry builtin load failed; \
                 registered runtime-registry agents only"
            );
        }
    }

    let registry_diagnostics = capability_registry.diagnostics();
    let registry_snapshot = capability_registry.snapshot();

    // Validation/projection pass (issue #4249, Workstream 10.1): exercise the
    // model/tool projection helpers that are slated to eventually replace the
    // live `harness.register_model`/`register_tool` glue. Today they are
    // infallible projections — they cannot themselves surface diagnostics — so
    // invoking them here is a non-fatal cross-check that every registered model
    // and tool projects into a harness registry cleanly. The authoritative,
    // fail-closed health signal stays `capability_registry.diagnostics()`
    // (captured in `registry_diagnostics` above and enforced by 10.2); a benign
    // projection difference must never abort a turn, so nothing here is folded
    // into that stream. The harness is deliberately NOT switched over to these
    // projections yet — that glue swap is explicitly deferred.
    let projected_models = capability_registry.to_model_registry();
    let projected_tools = capability_registry.to_tool_registry();
    tracing::debug!(
        models = projected_models.names().len(),
        tools = projected_tools.names().len(),
        graphs = capability_registry.names(ComponentKind::Graph).len(),
        agents = registered_agents.len(),
        runtime_agents = runtime_agent_count,
        builtin_supplement_agents = builtin_supplement_count,
        diagnostics = registry_diagnostics.len(),
        "[registry] per-turn capability projection summary"
    );
    // SHADOW tool-exposure layer (issue #4249, 01.3 — dynamic exposure). Compose
    // the OpenHuman exposure policy as a crate-native selection layer
    // (ToolAllowlistMiddleware + a ContextualToolSelectionMiddleware built via
    // `inheriting`) and run it in shadow: it emits the exposure decision
    // event-native (`AgentEvent::ToolsFiltered`) and logs any divergence between
    // the crate layer's decision and the set OpenHuman actually registered as
    // callable — WITHOUT changing the callable set (byte-identical to today). The
    // ownership flip + deletion of `tool_filter.rs`/`tool_prep.rs` is the gated
    // follow-up once the `[tool-exposure]` divergence logs show parity. Tags encode
    // the OpenHuman run context (agent id / channel / scope) for the flip; the
    // name-based `inheriting` predicate does not consult them yet.
    let exposure_tags: Vec<String> = {
        let mut tags = vec![if subagent_scope.is_some() {
            "scope:subagent".to_string()
        } else {
            "scope:root".to_string()
        }];
        if let Some(scope) = &subagent_scope {
            tags.push(format!("agent:{}", scope.agent_id));
            tags.push(format!("task:{}", scope.task_id));
        }
        if let Some(enforcement) = &tool_policy {
            tags.push(format!("channel:{}", enforcement.channel));
            tags.push(format!("agent_def:{}", enforcement.agent_definition_id));
        }
        tags
    };
    harness.push_middleware(Arc::new(
        middleware::OpenHumanToolExposureShadowMiddleware::new(
            &candidate_names,
            allowed.as_ref(),
            exposure_tags,
        ),
    ));

    // Prompt-cache prefix protection (issue #4249, 03.2). First declare the turn's
    // stable prefix (system prompt + tool schemas) as `PromptSegment`s, then let
    // the crate `PromptCacheGuardMiddleware` diff the cacheable prefix across model
    // calls and record a `CacheLayoutEvent` when volatile content busts it.
    // `before_model` hooks run in registration order, so the segment stamper must
    // precede the guard; both run before the context middlewares below (they only
    // touch the volatile tail / tool bodies, never the stable prefix). The guard is
    // returned so the run loop can drain its events into the observability bridge —
    // the crate-native replacement for the deleted `CacheAlignMiddleware` warn-log
    // (C3: the warn-only shadow is gone; this guard is the sole owner).
    harness.push_middleware(Arc::new(middleware::PromptCacheSegmentMiddleware));
    let prompt_cache_guard = Arc::new(PromptCacheGuardMiddleware::new());
    harness.push_middleware(prompt_cache_guard.clone());

    // openhuman context concerns as graph middlewares (issue #4249): microcompact
    // tool-body clearing and the after-tool byte cap / payload summarizer.
    // Installed before the summarization/trim block below so `before_model` hooks
    // run microcompact → compress → trim. (KV-cache-prefix drift is handled above
    // by the crate `PromptCacheGuardMiddleware`; the warn-only CacheAlign shadow
    // was deleted in C3.) Tool-result caps read the SDK registry policy snapshot,
    // not the OpenHuman-side tool lookup.
    // Capture each tool call's real success + content before the harness folds the
    // result into a `Message::tool` that drops the failure flag, so the turn can
    // build honest per-call `ToolCallRecord`s (post-turn hooks + cap checkpoint).
    //
    // REVERSE-ORDER RULE (issue #4464): the crate runs `after_tool` in REVERSE
    // registration order, so the LATER a middleware is pushed the EARLIER its
    // `after_tool` runs. This capture must observe the FINAL (summarized/capped)
    // content, so it is pushed BEFORE `context_mw.install` (which registers the
    // handoff + tool-output budget/caps) — that way its `after_tool` runs AFTER
    // those caps, not before. Registering it after `install` (the pre-#4464 bug)
    // made its `after_tool` run first and record the full raw payload of every
    // call, bloating the per-turn sink and feeding failure classification /
    // `ToolCallRecord.output_summary` pre-cap content.
    let tool_outcome_sink: ToolOutcomeSink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let failure_map: ToolFailureMap = Arc::default();
    harness.push_middleware(Arc::new(middleware::ToolOutcomeCaptureMiddleware::new(
        tool_outcome_sink.clone(),
        failure_map.clone(),
    )));

    let tool_policies = harness.tools().policies();
    context_mw.install(&mut harness, tool_policies);

    // Observe-only crate `BudgetMiddleware` (W2-budget-dedupe / workstream 06).
    // Installed with empty `BudgetLimits` so it NEVER enforces or halts: its
    // `before_model` preflight has no configured limit to trip, and its
    // `after_model` only folds each call's usage into its shared `BudgetTracker`.
    // It also re-emits `AgentEvent::UsageRecorded` per call (on top of the
    // runtime's own emit); the event bridge dedupes those by model-call iteration
    // so the global cost tracker still records each call exactly once (see
    // `observability::OpenhumanEventBridge::record_usage`). Enforcement STAYS with
    // the local `CostBudgetMiddleware` below (authoritative: reads the global
    // daily/monthly `CostTracker`).
    //
    // FLIP CRITERIA — what must hold before the crate `BudgetMiddleware` becomes
    // the enforcing owner and the local `CostBudgetMiddleware` + the
    // `agent/harness/turn_subagent_usage.rs` task-local are DELETED (deletion
    // ledger row: "crate-internal CostBudgetMiddleware + turn_subagent_usage.rs
    // task-local", `docs/tinyagents-full-migration-plan/99-deletion-ledger.md`):
    //   1. ≥ 500 production turns across BOTH parent and sub-agent runs with
    //      ZERO `[budget_shadow]` divergence log lines — proving the crate
    //      tracker's per-run token accounting matches the authoritative runtime
    //      `AgentRun.usage` on every model call.
    //   2. A pricing table wired via `BudgetMiddleware::with_pricing(..)` at
    //      parity with `cost::catalog::estimate_cost_usd`, so the crate can own
    //      MONEY (USD) budgets. Today the shadow compares TOKENS only (the
    //      observe-only crate middleware has no pricing, so its cost stays $0)
    //      and the local gate is the sole money-budget authority.
    //   3. Run-tree rollup wired: the same shared `BudgetTracker` handed to every
    //      sub-agent harness so a parent budget halts a recursive run pre-spend —
    //      replacing the `turn_subagent_usage` parent-turn rollup (06-cost step 3
    //      / 07.2 TaskStore rollup).
    // Until all three hold, this middleware is observe-only and the local gate
    // enforces.
    let shadow_budget = Arc::new(BudgetMiddleware::new(BudgetLimits::default()));
    let shadow_budget_tracker = shadow_budget.tracker();
    harness.push_middleware(shadow_budget);

    // Pre-call cost budget gate (issue #4249, Phase 5) — AUTHORITATIVE
    // enforcement: fail before a model call when OpenHuman's daily/monthly cost
    // budget is already exceeded. Self-gating — a no-op unless cost budgets are
    // configured. Demoted to a divergence-logging shadow owner (W2-budget-dedupe):
    // it keeps enforcing exactly as before, but ALSO compares its per-run token
    // accounting against the observe-only crate `BudgetMiddleware` above at end of
    // run and logs `[budget_shadow]` parity/divergence.
    harness.push_middleware(Arc::new(middleware::CostBudgetMiddleware::with_shadow(
        shadow_budget_tracker,
    )));

    // Autocompaction parity: when the provider's context window is known, install
    // the two-stage context-management step (issue #4249).
    //
    // 1. `ContextCompressionMiddleware` — the **summarization** step. Once the
    //    running token estimate crosses `window * SUMMARIZE_THRESHOLD_FRACTION`
    //    (90% of *this model's* context window), it folds the older slice of the
    //    transcript into a single LLM-generated system summary (keeping system
    //    messages + the recent window verbatim). This is keyed to whatever model
    //    the turn is running on, preserving the legacy context threshold.
    // 2. `ImageAwareMessageTrimMiddleware` — a deterministic, no-extra-LLM-call
    //    hard cap (issue #4462; replaces the crate `MessageTrimMiddleware`).
    //    Pushed **after** compression (so `before_model` runs compression first),
    //    it front-trims to the legacy proportional budget only as a last resort
    //    when even the summary + recent window still overflow — image markers
    //    priced flat, system messages never dropped, evictions logged.
    //
    // The LLM summarization step honors the `[context].enabled` /
    // `autocompact_enabled` opt-outs (a disabled config must not spend summarizer
    // tokens or rewrite history); the deterministic trim backstop always installs
    // when a window is known, matching the legacy always-on `trim_history` cap.
    // Concrete handle to the compression middleware (when installed), retained so
    // the run loop can drain its `records()` after the drive future returns and
    // surface each compaction's provenance (source ids + before/after token
    // estimates) — the `AgentEvent::Compressed` projection only carries the token
    // deltas, so provenance would otherwise be dropped (issue #4249, 03.1 item 6).
    let mut compression_mw: Option<Arc<ContextCompressionMiddleware>> = None;
    if let Some(window) = context_window.filter(|w| *w > 0) {
        if autocompact_enabled {
            let policy = summarize::summarization_policy(window);
            // Wrap the LLM-backed summarizer in a fault-tolerant, per-turn-caching
            // adapter (issue #4461): a summarizer failure must no longer abort the
            // turn (warn + circuit-breaker + deterministic trim instead), and an
            // identical re-issued input slice must not re-run the summarizer LLM.
            let summarizer = summarize::FaultTolerantCachingSummarizer::new(
                Box::new(summarize::ProviderModelSummarizer::new(
                    summarizer_model,
                    model,
                )),
                &policy,
            );
            let mw = Arc::new(ContextCompressionMiddleware::with_summarizer(
                policy,
                Box::new(summarizer),
            ));
            harness.push_middleware(mw.clone());
            compression_mw = Some(mw);
        }

        // Deterministic hard-cap trim (issue #4462). The crate
        // `MessageTrimMiddleware` regressed three legacy `token_budget.rs`
        // guards: it priced a base64 image at ~2M tokens (chars/4) and could
        // evict system messages, it reordered system messages to the front, and
        // its budget was the fixed `window − AGENT_TURN_MAX_OUTPUT_TOKENS`
        // (floored 1024) that collapses an 8k local model's input budget from
        // ~7373 to 1024. Our seam-owned `ImageAwareMessageTrimMiddleware`
        // restores all three: image markers priced at a flat cost, the
        // proportional reply reserve, system messages always kept in place, and a
        // grep-able warn with drop/token counts on any eviction.
        harness.push_middleware(Arc::new(
            middleware::ImageAwareMessageTrimMiddleware::for_context_window(window),
        ));
    }

    // SDK-owned tool-policy projection (issue #4249 / tinyagents-full-migration
    // 01.1). Keep this narrow for now: enforce sandbox requirements declared by
    // adapter policies without enabling classification/approval/result-byte
    // gates yet. `require_classification(true)` would currently reject an
    // unregistered hallucinated tool in `before_tool` before
    // `RunPolicy::unknown_tool` can return a recoverable tool error, while
    // OpenHuman's existing wrappers still own HITL approval and output caps.
    harness.push_middleware(Arc::new(
        TaToolPolicyMiddleware::new(harness.tools().policies()).require_sandbox(true),
    ));

    // Schema-guard (issue #4451): the crate runs a **fatal** JSON-schema gate on
    // every tool call between `before_tool` and the tool-wrap onion — a missing
    // required field / wrong type / bad enum returns `TinyAgentsError::Validation`
    // and aborts the whole turn (`chat_error`). This middleware re-runs the same
    // validation in `before_tool`; on failure it records a descriptive error and
    // rewrites the args to a schema-satisfying stub (so the crate gate passes),
    // then its `wrap_tool` hook short-circuits the flagged call with a synthetic
    // failed `ToolResult` before the stub can reach the tool — restoring the
    // legacy engine's "bad args → recoverable tool error the model self-corrects
    // on" behaviour. Installed as the **outermost** tool wrap so an invalid call
    // becomes a tool error before approval/policy wraps ever see the stub args.
    let schema_guard = Arc::new(middleware::SchemaGuardMiddleware::new(tool_sets.clone()));
    harness.push_tool_middleware(schema_guard.clone());

    // Human-in-the-loop approval as a named tool middleware (issue #4249,
    // Phase 1): an external-effect tool intercepts through the global
    // `ApprovalGate`, a denial short-circuits with a model-consumable result, and
    // an approved call records a terminal audit row. Replaces the inline approval
    // block that used to live in `execute_openhuman_tool`.
    harness.push_tool_middleware(Arc::new(middleware::ApprovalSecurityMiddleware::new(
        tool_sets.clone(),
    )));

    // CLI/RPC-only scope gate — a tool restricted to explicit CLI/RPC invocation
    // must not run from the model loop. Intrinsic to the tool, so installed on
    // every path (channel/session/sub-agent).
    harness.push_tool_middleware(Arc::new(middleware::CliRpcOnlyMiddleware::new(
        tool_sets.clone(),
    )));

    // Builder-configured tool policy (`.tool_policy()`), enforced at the tool
    // boundary. The in-house engine ran this in `agent_tool_exec`; the tinyagents
    // path bypassed it, so a deny/require-approval silently no-opped (security
    // regression). Installed only when the caller threads an enforcement context
    // (the session chat path); channel/CLI + sub-agent paths pass `None`.
    if let Some(enforcement) = tool_policy {
        harness.push_tool_middleware(Arc::new(middleware::ToolPolicyMiddleware::new(
            enforcement.policy,
            enforcement.session,
            tool_sets.clone(),
            enforcement.session_id,
            enforcement.channel,
            enforcement.agent_definition_id,
        )));
    }

    // Credential scrubbing (issue #4453): redact credential-shaped secrets out of
    // every tool result. The legacy engine ran `scrub_credentials` over every
    // tool output before it entered model context; the tinyagents path dropped
    // that call site. Installed as the **innermost** tool wrap (pushed last) so
    // it scrubs the RAW tool result before any outer wrap, the `after_tool`
    // chain (summarization/caps), the transcript push, or the tool-outcome
    // capture sink can observe the unredacted content — covering the parent,
    // sub-agent, persisted-transcript, and `ToolCallOutcome` surfaces by
    // construction since every path shares this seam.
    harness.push_tool_middleware(Arc::new(middleware::CredentialScrubMiddleware::new()));

    // Malformed-argument recovery (`before_tool`): repair a call's non-object
    // arguments before the crate's schema gate — decode JSON-encoded-string args
    // (optionally markdown-fenced) to an object, or coerce to `{}` only when the
    // tool schema has no required fields (engine parity). A non-object against a
    // required-field schema is left untouched so the schema-guard tool-error path
    // handles it instead of forcing a fatal `"<field> is required"` abort. Runs
    // before `SchemaGuardMiddleware::before_tool` (registered next) validates.
    harness.push_middleware(Arc::new(middleware::ArgRecoveryMiddleware::new(
        tool_sets.clone(),
    )));

    // Schema-guard `before_tool` (see the tool-wrap registration above): runs the
    // crate's schema validation and, on failure, flags the call + stubs its args
    // so the fatal gate passes and `wrap_tool` can short-circuit it. Registered
    // last so it validates the arguments `ArgRecoveryMiddleware` just repaired.
    harness.push_middleware(schema_guard);

    AssembledTurnHarness {
        harness,
        cursor,
        tool_names,
        failure_map,
        provider_usage_carry,
        error_slot,
        halt_summary,
        tool_outcome_sink,
        handle,
        early_exit_hook,
        tool_count,
        registry_snapshot,
        registry_diagnostics,
        tool_result_artifact_index,
        compression_mw,
        prompt_cache_guard,
    }
}

/// Feed an **unobserved** turn's aggregate usage into the global cost tracker.
///
/// The per-call tracker feed lives in the event bridge
/// ([`OpenhumanEventBridge::record_usage`]), which only exists on observed runs
/// (`on_progress` set). Without this aggregate record a fire-and-forget turn's
/// spend never reaches the cost dashboard / wallet surfaces (issue #4249,
/// Phase 5 rollup gap). The bridge and this fallback are mutually exclusive,
/// so spend is recorded exactly once either way.
///
/// Returns `true` when a record was attempted (any tokens observed); all-zero
/// usage is skipped so providers that echo no usage don't inflate the request
/// count. Recording is best-effort — a missing/uninitialised tracker is a
/// silent no-op by contract.
fn record_unobserved_turn_usage(
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
    crate::openhuman::cost::record_provider_usage(
        model,
        &crate::openhuman::inference::provider::UsageInfo {
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

#[cfg(test)]
mod tests;
