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

mod convert;
pub(crate) mod delegation;
mod embeddings;
pub(crate) mod journal;
pub(crate) mod middleware;
mod model;
pub(crate) mod observability;
pub(crate) mod orchestration;
pub(crate) mod payload_summarizer;
pub(crate) mod retriever;
mod routes;
mod run_cancellation_context;
pub(crate) mod stop_hooks;
pub(crate) mod subagent_graph;
mod summarize;
mod tools;
mod topology;

use std::sync::Arc;

use anyhow::Result;
use tinyagents::harness::cache::InMemoryResponseCache;
use tinyagents::harness::context::{RunConfig, RunContext};
use tinyagents::harness::events::EventSink;
use tinyagents::harness::message::Message as TaMessage;
use tinyagents::harness::middleware::{
    ContextCompressionMiddleware, MessageTrimMiddleware, PromptCacheGuardMiddleware,
    ToolPolicyMiddleware as TaToolPolicyMiddleware,
};
use tinyagents::harness::model::CapabilitySet;
use tinyagents::harness::runtime::{AgentHarness, RunPolicy, UnknownToolPolicy};
use tinyagents::harness::steering::{SteeringCommand, SteeringHandle};
use tinyagents::harness::store::StoreRegistry;
use tinyagents::harness::summarization::TrimStrategy;
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
use model::ThinkingForwarder;

#[allow(unused_imports)] // Wired into the recall/retrieval facade in workstream 09.2.
pub(crate) use embeddings::ProviderEmbeddingModel;
pub(crate) use middleware::{HandoffConfig, SuperContextConfig, TurnContextMiddleware};
use model::ProviderModel;
pub(crate) use observability::SubagentScope;
use observability::{CapPauser, IterationCursor, OpenhumanEventBridge, ToolNameMap};
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
/// The recursion depth cap is also set here so TinyAgents uses OpenHuman's
/// existing sub-agent spawn depth instead of the SDK default.
/// Retry is set to a single attempt: the openhuman [`Provider`] already does its
/// own internal retry/backoff (via the still-wrapped `ReliableProvider`), so a
/// second harness-level retry layer would double-retry transient errors and,
/// worse, swallow a deterministic provider error when a mock/test provider yields
/// a different result on the retry. This pin stays until `ReliableProvider` is
/// un-wrapped in the 02.2 conformance pass (Workstream 11); the crate's
/// exp-backoff [`RetryPolicy`](tinyagents::harness::retry::RetryPolicy) fields
/// stay at the default schedule so raising `max_attempts` later is a one-line flip.
///
/// Cross-route **fallback** (`RunPolicy.fallback`) is orthogonal to retry and is
/// populated per-turn by the caller ([`assemble_turn_harness`] via
/// [`routes::route_fallback_policy`]); it is safe to enable now because
/// `ReliableProvider` does *not* fail over across the registered workload-tier
/// routes (chat→burst, reasoning→agentic, …) the way the harness registry can.
fn run_policy_for(max_iterations: usize, response_cache_enabled: bool) -> RunPolicy {
    let mut policy = RunPolicy::default();
    policy.limits.max_model_calls = max_iterations;
    policy.limits.max_tool_calls = max_iterations.saturating_mul(8).max(8);
    policy.limits.max_depth = MAX_SPAWN_DEPTH;
    policy.retry.max_attempts = 1;
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
pub(crate) async fn run_turn_via_tinyagents_shared(
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
    workspace_descriptor: Option<WorkspaceDescriptor>,
    deterministic_cacheable: bool,
) -> Result<TinyagentsTurnOutcome> {
    // `0` means "unset" → the legacy default (a native-bus / test convention);
    // otherwise the harness model-call cap would be zero and abort the run before
    // the first provider call.
    let max_iterations = effective_max_iterations(max_iterations);
    let AssembledTurnHarness {
        harness,
        cursor,
        tool_names,
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
        provider,
        model,
        temperature,
        tool_sets,
        allowed,
        max_iterations,
        on_progress.clone(),
        subagent_scope.clone(),
        context_window,
        early_exit_tools,
        max_output_tokens,
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

    let config = RunConfig::new("agent_turn")
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
    if let Some(index) = tool_result_artifact_index {
        let mut stores = StoreRegistry::new();
        stores.register(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE, index);
        ctx = ctx.with_stores(stores);
    }

    let streaming = on_progress.is_some();
    // Retain a clone of the progress sink so the turn can emit a terminal
    // `TurnCompleted` after the run (the harness event stream the bridge mirrors
    // has no run-completed event). Parent turns only — a sub-agent turn reports
    // via its `Subagent*` events, not a top-level `TurnCompleted`.
    let turn_completed_sink = subagent_scope
        .is_none()
        .then(|| on_progress.clone())
        .flatten();
    // A sink is needed to mirror progress (bridge), to observe model-call
    // completions for the cap pauser, or to persist a durable event journal
    // (issue #4249, 05.1). The journal must attach even for an unobserved
    // (`on_progress = None`) turn so the run stays reconstructable, so the
    // EventSink is now created unconditionally — cheap (an empty sink) and, if
    // no consumer subscribes, inert.
    let events = Some(EventSink::new());

    let bridge = match (&events, on_progress) {
        (Some(events), Some(tx)) => {
            let bridge = OpenhumanEventBridge::with_scope(
                Some(tx),
                model,
                max_iterations,
                subagent_scope.clone(),
                cursor.clone(),
                tool_names.clone(),
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

    // Durable event journal + status store (issue #4249, 05.1). Attached *in
    // addition to* the bridge above: the EventSink fans out to both, so the
    // existing progress/global-bus path is untouched. Best-effort and non-fatal
    // — a failure to open/attach the journal returns `None` and the turn runs
    // unaffected. The handle stamps the terminal status once the run returns.
    let turn_journal = match &events {
        Some(events) => journal::attach_turn_journal(events, model).await,
        None => None,
    };

    if let Some(events) = &events {
        ctx = ctx.with_events(events.clone());
    }

    // Steering: attach the shared handle (when present), drain any already-queued
    // steer messages into it (so a pre-run steer lands before the first model
    // call), and forward mid-flight steers via a poller aborted when the run
    // returns. The same handle carries the early-exit `Pause`.
    let mut registered_steering_task_id = None;
    let steering_forwarder = if let Some(handle) = handle {
        if let Some(scope) = &subagent_scope {
            let task_id = orchestration::TaskId::new(scope.task_id.clone());
            orchestration::shared_steering_registry().register(task_id.clone(), handle.clone());
            tracing::debug!(
                task_id = scope.task_id.as_str(),
                "[tinyagents] registered subagent steering handle"
            );
            registered_steering_task_id = Some(task_id);
        }
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
    let run_result = with_run_cancellation(cancellation.clone(), async {
        if streaming {
            Box::pin(harness.invoke_streaming_in_context(&(), ctx, input)).await
        } else {
            Box::pin(harness.invoke_in_context(&(), ctx, input)).await
        }
    })
    .await;
    if let Some(forwarder) = steering_forwarder {
        forwarder.abort();
    }
    if let Some(task_id) = registered_steering_task_id {
        orchestration::shared_steering_registry().deregister(&task_id);
        tracing::debug!(
            task_id = task_id.as_str(),
            "[tinyagents] deregistered subagent steering handle"
        );
    }
    let run = match run_result {
        Ok(run) => run,
        Err(e) => {
            // Durable journal: stamp the terminal failed status (best-effort,
            // non-fatal) before unwinding through the typed-error mapping below.
            if let Some(journal) = &turn_journal {
                journal.finish_failed(&e.to_string()).await;
            }
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
            if let Some(depth_err) = tinyagents_depth_error(&e) {
                return Err(anyhow::Error::new(depth_err));
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
    // content silently busting the provider KV-cache prefix. The structured
    // successor to `CacheAlignMiddleware`'s free-text warn-log (still installed in
    // parallel until parity is shown).
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
    /// [`observability::surface_cache_layout_events`] — the structured successor to
    /// the `CacheAlignMiddleware` warn-log.
    prompt_cache_guard: Arc<PromptCacheGuardMiddleware>,
}

/// Assemble the turn harness for [`run_turn_via_tinyagents_shared`]: register
/// the provider model, every shared tool, and the full middleware stack in the
/// intended order. Split out of the runner so the adapter inventory is directly
/// testable (issue #4249, Phase 11) — the returned [`AssembledTurnHarness`]
/// exposes the harness registries without driving a run.
#[allow(clippy::too_many_arguments)]
fn assemble_turn_harness(
    provider: Arc<dyn Provider>,
    model: &str,
    temperature: f64,
    tool_sets: Vec<Arc<Vec<Box<dyn crate::openhuman::tools::Tool>>>>,
    allowed: HashSet<String>,
    max_iterations: usize,
    on_progress: Option<Sender<AgentProgress>>,
    subagent_scope: Option<SubagentScope>,
    context_window: Option<u64>,
    early_exit_tools: &[&str],
    max_output_tokens: Option<u32>,
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
    // Record the model's context window on its capability profile (issue #4249,
    // Phase 2) so the crate can validate input capacity before dispatch.
    if let Some(window) = context_window.filter(|w| *w > 0) {
        provider_model = provider_model.with_context_window(window);
    }
    if let Some(tx) = &on_progress {
        provider_model = provider_model.with_thinking(ThinkingForwarder::new(
            tx.clone(),
            subagent_scope.clone(),
            cursor.clone(),
            tool_names.clone(),
        ));
    }
    // Recover the original (downcastable) provider error if the run fails — the
    // harness only carries a stringified copy.
    let error_slot = provider_model.error_slot();
    let provider_model = Arc::new(provider_model);
    capability_registry.replace_model(model, provider_model.clone());
    harness
        .register_model(model, provider_model)
        .set_default_model(model);

    // Project the full workload-route set into the registry (issue #4249,
    // Workstream 02.1). Each route is an additive registry entry carrying its
    // per-route capability profile; `set_default_model` above keeps the turn's
    // effective model as the dispatch target, so behavior is preserved until
    // fallback/selection (02.2) chooses among the routes. `summary_provider` is
    // the retained provider handle (the other clone was consumed into the
    // primary `ProviderModel`); `build_route_models` clones it per route and
    // skips the turn's own model so we don't shadow the default.
    for route in
        routes::build_route_models(&summary_provider, temperature, model, max_output_tokens)
    {
        let routes::RouteModel {
            name,
            model: route_model,
        } = route;
        capability_registry.replace_model(name.as_str(), route_model.clone());
        harness.register_model(name, route_model);
    }

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
    let mut seen_candidates: HashSet<String> = HashSet::new();
    let candidate_names: Vec<String> = tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .map(|tool| tool.name())
        .filter_map(|name| {
            seen_candidates
                .insert(name.to_string())
                .then(|| name.to_string())
        })
        .collect();
    let mut registered: HashSet<String> = HashSet::new();
    for name in candidate_names.iter().map(String::as_str) {
        if !registered.contains(name) && (allowed.is_empty() || allowed.contains(name)) {
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
            &allowed,
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
    // the structured successor to `CacheAlignMiddleware`'s warn-log (kept installed
    // via `context_mw` until parity is shown).
    harness.push_middleware(Arc::new(middleware::PromptCacheSegmentMiddleware));
    let prompt_cache_guard = Arc::new(PromptCacheGuardMiddleware::new());
    harness.push_middleware(prompt_cache_guard.clone());

    // openhuman context concerns as graph middlewares (issue #4249): cache-align
    // warnings, microcompact tool-body clearing, and the after-tool byte cap /
    // payload summarizer. Installed before the summarization/trim block below so
    // `before_model` hooks run cache-align → microcompact → compress → trim.
    // Tool-result caps read the SDK registry policy snapshot, not the
    // OpenHuman-side tool lookup.
    let tool_policies = harness.tools().policies();
    context_mw.install(&mut harness, tool_policies);

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
    //    the turn is running on, preserving the legacy context threshold.
    // 2. `MessageTrimMiddleware` — a deterministic, no-extra-LLM-call hard cap.
    //    Pushed **after** compression (so `before_model` runs compression first),
    //    it front-trims to budget only as a last resort when even the summary +
    //    recent window still overflow.
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
            let mw = Arc::new(ContextCompressionMiddleware::with_summarizer(
                summarize::summarization_policy(window),
                Box::new(summarize::ProviderModelSummarizer::new(
                    summary_provider,
                    model,
                    temperature,
                )),
            ));
            harness.push_middleware(mw.clone());
            compression_mw = Some(mw);
        }

        let budget = window.saturating_sub(
            crate::openhuman::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS as u64,
        );
        harness.push_middleware(Arc::new(MessageTrimMiddleware::new(
            TrimStrategy::MaxTokens(budget.max(1024)),
        )));
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
            enforcement.session_id,
            enforcement.channel,
            enforcement.agent_definition_id,
        )));
    }

    // Malformed-argument recovery (`before_tool`): coerce a call's non-object
    // arguments (invalid JSON parses to Null) to `{}` so a single bad tool call is
    // recoverable — the harness would otherwise reject it against an object schema
    // and abort the whole turn. Engine parity.
    harness.push_middleware(Arc::new(middleware::ArgRecoveryMiddleware));

    AssembledTurnHarness {
        harness,
        cursor,
        tool_names,
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
