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
//! the [`OpenhumanEventBridge`] mirrors the 0.2.0 harness event stream onto
//! `AgentProgress` (live tool timeline, incremental text deltas, cost footer),
//! [`ProviderModel::stream`] forwards true token streaming, multimodal markers
//! are expanded, and history is trimmed to the context window. Mid-flight
//! steering, sub-agent child-progress deltas (incl. thinking), and the
//! `ask_user_clarification` early-exit pause are all re-wired onto 0.2.0.

pub mod checkpoint;
mod convert;
pub mod delegation;
pub mod middleware;
mod model;
pub mod observability;
pub mod orchestration;
pub mod stop_hooks;
pub mod summarize;
mod tools;
pub mod topology;

use std::sync::Arc;

use anyhow::Result;
use tinyagents::harness::context::{RunConfig, RunContext};
use tinyagents::harness::events::EventSink;
use tinyagents::harness::message::Message as TaMessage;
use tinyagents::harness::middleware::{ContextCompressionMiddleware, MessageTrimMiddleware};
use tinyagents::harness::runtime::{AgentHarness, RunPolicy};
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::summarization::TrimStrategy;

use crate::openhuman::agent::harness::run_queue::RunQueue;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::inference::provider::{ChatMessage, ConversationMessage, Provider};

pub use checkpoint::SqlRunLedgerCheckpointer;
pub use middleware::{HandoffConfig, SuperContextConfig, TurnContextMiddleware};
pub use model::{ProviderModel, ThinkingForwarder};
pub use observability::{CapPauser, IterationCursor, OpenhumanEventBridge, SubagentScope};
pub use tools::{
    EarlyExit, EarlyExitHook, SharedToolAdapter, ToolAdapter, UnknownToolAdapter,
    UNKNOWN_TOOL_SENTINEL,
};

use std::collections::HashSet;
use std::sync::Arc as StdArc;
use tokio::sync::mpsc::Sender;

/// The builder-configured [`ToolPolicy`](crate::openhuman::agent::tool_policy::ToolPolicy)
/// plus the session context a policy check needs, handed to the shared turn seam
/// so it can install the [`ToolPolicyMiddleware`](middleware::ToolPolicyMiddleware).
/// `None` means "no policy enforcement on this turn" (the channel/CLI + sub-agent
/// paths, which carry their own gating).
pub struct ToolPolicyEnforcement {
    pub policy: StdArc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    /// The session's channel-permission snapshot — enforces the per-channel
    /// permission ceiling (deny + per-call permission-level gate) the in-house
    /// engine ran in `agent_tool_exec`.
    pub session: crate::openhuman::agent_tool_policy::ToolPolicySession,
    pub session_id: String,
    pub channel: String,
    pub agent_definition_id: String,
}

/// Drain the run queue's pending steer messages and forward them to the
/// tinyagents [`SteeringHandle`] as injected user turns (the harness applies
/// them to the working transcript at the next iteration checkpoint). This is the
/// bridge behind the `steer_subagent` / mid-flight-steering feature.
async fn forward_steers(queue: &RunQueue, handle: &SteeringHandle) {
    for msg in queue.drain_steers().await {
        handle.send(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "[User steering message]: {}",
            msg.text
        ))));
    }
}

/// Forward any queued **collect** messages (orchestrator/monitor lines enqueued
/// via `QueueMode::Collect`) into the run as injected user turns so they reach the
/// next LLM call as additional context. The in-house loop drained these each
/// iteration (`drain_collects`); the tinyagents rewrite wired only `forward_steers`
/// (issue #4249), so monitor lines never reached the model. Mirrors the legacy
/// `[Additional context from user]:` framing the model was taught to read.
async fn forward_collects(queue: &RunQueue, handle: &SteeringHandle) {
    for msg in queue.drain_collects().await {
        handle.send(SteeringCommand::InjectMessage(TaMessage::user(format!(
            "[Additional context from user]: {}",
            msg.text
        ))));
    }
}

/// Build the harness [`RunPolicy`] for an openhuman turn.
///
/// The loop enforces limits from `self.policy.limits` (not the per-run
/// `RunConfig`), so the model-call cap **must** be set here or it falls back to
/// the tinyagents default of 25 — far more than openhuman's `max_iterations`.
/// Retry is set to a single attempt: the openhuman [`Provider`] already does its
/// own internal retry/backoff, so a second harness-level retry layer would
/// double-retry transient errors and, worse, swallow a deterministic provider
/// error when a mock/test provider yields a different result on the retry.
fn run_policy_for(max_iterations: usize) -> RunPolicy {
    let mut policy = RunPolicy::default();
    policy.limits.max_model_calls = max_iterations;
    policy.limits.max_tool_calls = max_iterations.saturating_mul(8).max(8);
    policy.retry.max_attempts = 1;
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
pub struct TinyagentsTurnOutcome {
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
    /// final answer — the tinyagents analogue of `CheckpointStrategy::on_max_iter`.
    pub hit_cap: bool,
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
pub struct ToolCallOutcome {
    pub call_id: String,
    pub name: String,
    pub success: bool,
    pub content: String,
}

/// Shared sink the [`ToolOutcomeCaptureMiddleware`](middleware::ToolOutcomeCaptureMiddleware)
/// appends each tool call's outcome to, drained into the turn outcome.
pub type ToolOutcomeSink = std::sync::Arc<std::sync::Mutex<Vec<ToolCallOutcome>>>;

/// Shared slot the repeated-failure breaker writes a root-cause halt summary into
/// when it trips. The turn overrides its final text with this summary so the
/// no-progress halt surfaces the cause instead of an empty/last-model reply
/// (legacy `RepeatFailureGuard` parity).
pub type HaltSummarySlot = std::sync::Arc<std::sync::Mutex<Option<String>>>;

/// Drive an agent turn through the `tinyagents` agent-loop harness.
///
/// Registers `provider` as the default model and every entry in `resolved_tools`
/// as a harness tool, seeds the loop with `history`, and runs the loop bounded
/// by `max_iterations` model calls. Returns the final text plus the resulting
/// transcript translated back to openhuman [`ChatMessage`]s.
pub async fn run_turn_via_tinyagents(
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
    harness.with_policy(run_policy_for(max_iterations));
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
        .with_max_tool_calls(max_iterations.saturating_mul(8).max(8));

    tracing::info!(
        model,
        max_iterations,
        tools = tool_count,
        "[tinyagents] routing agent turn through tinyagents harness"
    );

    let input = convert::history_to_messages(&history);
    // Box the (large) harness drive future — see `run_turn_via_tinyagents_shared`.
    let run = match Box::pin(harness.invoke(&(), (), config, input)).await {
        Ok(run) => run,
        Err(e) => {
            if let Some(original) = error_slot.lock().unwrap().take() {
                return Err(original);
            }
            return Err(anyhow::anyhow!("tinyagents harness run failed: {e}"));
        }
    };

    let text = run.text().unwrap_or_default();
    let out_history = convert::messages_to_history(&run.messages);
    let conversation =
        convert::messages_to_conversation(convert::messages_since_last_user(&run.messages));

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
/// `allowed` is the callable tool-name whitelist (empty = every tool visible in
/// `tool_sets`); each callable tool is advertised via its own `spec()`.
///
/// When `on_progress` is `Some`, the run streams (`invoke_streaming_in_context`)
/// and a [`OpenhumanEventBridge`] mirrors the harness event stream onto
/// `AgentProgress` (live tool timeline, text deltas, cost/token footer) and the
/// global cost tracker — restoring the seams the legacy `run_turn_engine`
/// produced. Pass `None` for fire-and-forget turns (channel/sub-agent) that
/// only need the final text.
///
/// When `context_window` is known, a [`MessageTrimMiddleware`] keeps history
/// under budget (autocompaction parity).
///
/// `run_queue` forwards mid-flight steer messages into the run; `subagent_scope`
/// re-scopes progress to the `Subagent*` variants (child runs); `early_exit_tools`
/// name the tools that pause the loop (e.g. `ask_user_clarification`) and surface
/// the question via [`TinyagentsTurnOutcome::early_exit_tool`].
#[allow(clippy::too_many_arguments)]
pub async fn run_turn_via_tinyagents_shared(
    provider: Arc<dyn Provider>,
    model: &str,
    temperature: f64,
    history: Vec<ChatMessage>,
    tool_sets: Vec<Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>>,
    allowed: HashSet<String>,
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
) -> Result<TinyagentsTurnOutcome> {
    // `0` means "unset" → the legacy default (a native-bus / test convention);
    // otherwise the harness model-call cap would be zero and abort the run before
    // the first provider call.
    let max_iterations = effective_max_iterations(max_iterations);
    let mut harness: AgentHarness<()> = AgentHarness::new();
    harness.with_policy(run_policy_for(max_iterations));

    // Shared 1-based model-call cursor: the event bridge advances it on each
    // model start; the model adapter reads it to attribute out-of-band thinking
    // deltas (tinyagents has no reasoning channel on its stream).
    // The set of tool names the model may call: every advertised tool plus the
    // unknown-tool sentinel. A call outside it is rewritten onto the sentinel so
    // a hallucinated tool recovers instead of aborting the run — enforced by the
    // `UnknownToolRewriteMiddleware` (`before_tool`) installed below.
    let valid_tools: Arc<HashSet<String>> = {
        let mut names: HashSet<String> = tool_sets
            .iter()
            .flat_map(|set| set.iter())
            .map(|t| t.name().to_string())
            .filter(|name| allowed.is_empty() || allowed.contains(name))
            .collect();
        names.insert(UNKNOWN_TOOL_SENTINEL.to_string());
        Arc::new(names)
    };

    let cursor: IterationCursor = Arc::default();
    // Keep a provider handle for the context-window summarizer (the run consumes
    // the other clone into the `ProviderModel`).
    let summary_provider = provider.clone();
    let mut provider_model = ProviderModel::new(provider, model, temperature);
    // Cap the model's per-call output budget (parity with the legacy engine,
    // which bounded the main agent at `AGENT_TURN_MAX_OUTPUT_TOKENS` and each
    // sub-agent at its `max_turn_output_tokens`). Without this the tinyagents
    // path ran the provider uncapped.
    if let Some(cap) = max_output_tokens {
        provider_model = provider_model.with_max_tokens(cap);
    }
    if let Some(tx) = &on_progress {
        provider_model = provider_model.with_thinking(ThinkingForwarder::new(
            tx.clone(),
            subagent_scope.clone(),
            cursor.clone(),
        ));
    }
    // Recover the original (downcastable) provider error if the run fails — the
    // harness only carries a stringified copy.
    let error_slot = provider_model.error_slot();
    harness
        .register_model(model, Arc::new(provider_model))
        .set_default_model(model);

    // openhuman context concerns as graph middlewares (issue #4249): cache-align
    // warnings, microcompact tool-body clearing, and the after-tool byte cap /
    // payload summarizer. Installed before the summarization/trim block below so
    // `before_model` hooks run cache-align → microcompact → compress → trim.
    // Capture the autocompaction opt-out before `install` consumes `context_mw`.
    let autocompact_enabled = context_mw.autocompact_enabled;
    context_mw.install(&mut harness, &tool_sets);

    // Pre-call cost budget gate (issue #4249, Phase 5): fail before a model call
    // when OpenHuman's daily/monthly cost budget is already exceeded. Self-gating
    // — a no-op unless cost budgets are configured.
    harness.push_middleware(Arc::new(middleware::CostBudgetMiddleware));

    // Autocompaction parity: when the provider's context window is known, install
    // the two-stage context-management step (issue #4249).
    //
    // 1. `ContextCompressionMiddleware` — the **summarization** step. Once the
    //    running token estimate crosses `window * SUMMARIZE_THRESHOLD_FRACTION`
    //    (90% of *this model's* context window), it folds the older slice of the
    //    transcript into a single LLM-generated system summary (keeping system
    //    messages + the recent window verbatim). This is keyed to whatever model
    //    the turn is running on, mirroring the legacy `ContextGuard` threshold.
    // 2. `MessageTrimMiddleware` — a deterministic, no-extra-LLM-call hard cap.
    //    Pushed **after** compression (so `before_model` runs compression first),
    //    it front-trims to budget only as a last resort when even the summary +
    //    recent window still overflow.
    //
    // The LLM summarization step honors the `[context].enabled` /
    // `autocompact_enabled` opt-outs (a disabled config must not spend summarizer
    // tokens or rewrite history); the deterministic trim backstop always installs
    // when a window is known, matching the legacy always-on `trim_history` cap.
    if let Some(window) = context_window.filter(|w| *w > 0) {
        if autocompact_enabled {
            harness.push_middleware(Arc::new(ContextCompressionMiddleware::with_summarizer(
                summarize::summarization_policy(window),
                Box::new(summarize::ProviderModelSummarizer::new(
                    summary_provider,
                    model,
                    temperature,
                )),
            )));
        }

        let budget = window.saturating_sub(
            crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS as u64,
        );
        harness.push_middleware(Arc::new(MessageTrimMiddleware::new(
            TrimStrategy::MaxTokens(budget.max(1024)),
        )));
    }

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
    let handle = Some(SteeringHandle::allow_all());

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
    // sets (newest set wins on a name clash; `allowed` empty = all visible).
    let mut registered: HashSet<String> = HashSet::new();
    for name in tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .map(|t| t.name())
    {
        if !registered.contains(name) && (allowed.is_empty() || allowed.contains(name)) {
            if let Some(mut adapter) = SharedToolAdapter::for_name(tool_sets.clone(), name) {
                if early_exit_set.contains(name) {
                    if let Some(hook) = &early_exit_hook {
                        adapter = adapter.with_early_exit(hook.clone());
                    }
                }
                registered.insert(name.to_string());
                harness.register_tool(Arc::new(adapter));
            }
        }
    }
    // The unknown-tool sentinel: the model adapter rewrites any unadvertised tool
    // call onto it so the run recovers gracefully instead of aborting. Its wording
    // matches the legacy engine (sub-agent vs top-level).
    harness.register_tool(Arc::new(UnknownToolAdapter::new(subagent_scope.is_some())));
    let tool_count = registered.len();

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

    // Capture each tool call's real success + content before the harness folds the
    // result into a `Message::tool` that drops the failure flag, so the turn can
    // build honest per-call `ToolCallRecord`s (post-turn hooks + cap checkpoint).
    let tool_outcome_sink: ToolOutcomeSink = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    harness.push_middleware(Arc::new(middleware::ToolOutcomeCaptureMiddleware::new(
        tool_outcome_sink.clone(),
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
            allowed.clone(),
            enforcement.session_id,
            enforcement.channel,
            enforcement.agent_definition_id,
        )));
    }

    // Unknown-tool recovery as a `before_tool` middleware (issue #4249, Phase 1
    // Task B): a call to an unadvertised tool is rewritten onto the recovery
    // sentinel before the harness resolves it, so a hallucinated tool name is a
    // recoverable result rather than a fatal `ToolNotFound`. Replaces the
    // `valid_tools` rewrite that used to live in `ProviderModel`.
    harness.push_middleware(Arc::new(middleware::UnknownToolRewriteMiddleware::new(
        valid_tools,
    )));

    // Malformed-argument recovery (`before_tool`): coerce a call's non-object
    // arguments (invalid JSON parses to Null) to `{}` so a single bad tool call is
    // recoverable — the harness would otherwise reject it against an object schema
    // and abort the whole turn. Engine parity.
    harness.push_middleware(Arc::new(middleware::ArgRecoveryMiddleware));

    let config = RunConfig::new("agent_turn")
        .with_max_model_calls(max_iterations)
        .with_max_tool_calls(max_iterations.saturating_mul(8).max(8));

    tracing::info!(
        model,
        max_iterations,
        tools = tool_count,
        observed = on_progress.is_some(),
        "[tinyagents] routing turn through tinyagents harness (shared tools)"
    );

    let input = convert::history_to_messages(&history);

    // Build the run context: an optional event sink feeds the progress/cost
    // bridge (streaming) and/or the model-call-cap pauser; the shared steering
    // handle carries mid-flight, early-exit, and cap pauses.
    let mut ctx = RunContext::new(config, ());

    let streaming = on_progress.is_some();
    // Retain a clone of the progress sink so the turn can emit a terminal
    // `TurnCompleted` after the run (the harness event stream the bridge mirrors
    // has no run-completed event). Parent turns only — a sub-agent turn reports
    // via its `Subagent*` events, not a top-level `TurnCompleted`.
    let turn_completed_sink = subagent_scope
        .is_none()
        .then(|| on_progress.clone())
        .flatten();
    // A sink is needed to mirror progress (bridge) or to observe model-call
    // completions for the cap pauser.
    let events = (on_progress.is_some() || pause_at_cap).then(EventSink::new);

    let bridge = match (&events, on_progress) {
        (Some(events), Some(tx)) => {
            let bridge = OpenhumanEventBridge::with_scope(
                Some(tx),
                model,
                max_iterations,
                subagent_scope.clone(),
                cursor.clone(),
            );
            events.subscribe(bridge.clone());
            Some(bridge)
        }
        _ => None,
    };

    // Cap pauser: stop gracefully at the model-call budget (returning the partial
    // transcript) so the caller can summarize a checkpoint instead of erroring.
    if pause_at_cap {
        if let (Some(events), Some(handle)) = (&events, &handle) {
            events.subscribe(CapPauser::new(handle.clone(), max_iterations));
        }
    }

    if let Some(events) = &events {
        ctx = ctx.with_events(events.clone());
    }

    // Steering: attach the shared handle (when present), drain any already-queued
    // steer messages into it (so a pre-run steer lands before the first model
    // call), and forward mid-flight steers via a poller aborted when the run
    // returns. The same handle carries the early-exit `Pause`.
    let steering_forwarder = if let Some(handle) = handle {
        if let Some(queue) = run_queue.clone() {
            forward_steers(&queue, &handle).await;
            forward_collects(&queue, &handle).await;
        }
        ctx = ctx.with_steering(handle.clone());
        run_queue.map(|queue| {
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                    forward_steers(&queue, &handle).await;
                    forward_collects(&queue, &handle).await;
                }
            })
        })
    } else {
        None
    };

    // Heap-allocate the harness drive future. It is large (it owns the whole run
    // context, middleware stack, and loop state), and a sub-agent turn runs
    // nested inside its parent's drive future — leaving it inline on the stack
    // overflows when the parent + child drives compose. Boxing keeps only a
    // pointer on the stack at each level.
    let run_result = if streaming {
        Box::pin(harness.invoke_streaming_in_context(&(), ctx, input)).await
    } else {
        Box::pin(harness.invoke_in_context(&(), ctx, input)).await
    };
    if let Some(forwarder) = steering_forwarder {
        forwarder.abort();
    }
    let run = match run_result {
        Ok(run) => run,
        Err(e) => {
            // Prefer the original typed provider error (preserves `AgentError`
            // downcasts the caller relies on) over the harness's string wrap.
            if let Some(original) = error_slot.lock().unwrap().take() {
                return Err(original);
            }
            // The model-call cap (when not pausing gracefully — the channel/CLI
            // path) maps to the typed `AgentError::MaxIterationsExceeded` so
            // callers downcast it (Sentry skip) and render the canonical
            // "Agent exceeded maximum tool iterations" message, matching the
            // legacy `ErrorCheckpoint`.
            if let tinyagents::TinyAgentsError::LimitExceeded(msg) = &e {
                if msg.contains("model call") {
                    return Err(anyhow::Error::new(
                        crate::openhuman::agent::error::AgentError::MaxIterationsExceeded {
                            max: max_iterations,
                        },
                    ));
                }
            }
            return Err(anyhow::anyhow!("tinyagents harness run failed: {e}"));
        }
    };
    // Terminal turn event (parity with the legacy engine's `progress::emit`): the
    // harness stream has no run-completed event, so emit `TurnCompleted` here with
    // the model-call count as the iteration total. Parent turns only; best-effort.
    if let Some(sink) = &turn_completed_sink {
        let _ = sink.try_send(AgentProgress::TurnCompleted {
            iterations: run.model_calls as u32,
        });
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

    if let Some(summary) = breaker_halt {
        text = summary;
    }

    let tool_outcomes = tool_outcome_sink
        .lock()
        .map(|guard| guard.clone())
        .unwrap_or_default();

    Ok(TinyagentsTurnOutcome {
        text,
        history: convert::messages_to_history(&run.messages),
        conversation: convert::messages_to_conversation(convert::messages_since_last_user(
            &run.messages,
        )),
        model_calls: run.model_calls,
        tool_calls: run.tool_calls,
        input_tokens,
        output_tokens,
        cached_input_tokens,
        charged_amount_usd,
        early_exit_tool,
        hit_cap,
        tool_outcomes,
    })
}

#[cfg(test)]
mod tests;
