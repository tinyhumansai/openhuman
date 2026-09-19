//! Drive an openhuman agent turn through the `tinyagents` agent-loop harness.
//!
//! [`run_turn_via_tinyagents_shared`] is the entry point the channel/session/
//! sub-agent routes use; [`run_turn_via_tinyagents`] is a thin test-only
//! variant with no middleware stack.

#[cfg(test)]
use crate::agent::tinyagents::model::ProfileOverrideModel;
#[cfg(test)]
use crate::agent::tinyagents::model::TurnChatModel;
#[cfg(test)]
use crate::agent::tinyagents::turn_policy::run_policy_for;
use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use tinyagents_harness::agent_loop::AgentStreamItem;
use tinyagents_harness::context::RunConfig;
use tinyagents_harness::events::EventSink;
use tinyagents_harness::host::{ContextComposer, TurnContextRequest};
use tinyagents_harness::runtime::{
    AgentHarness, AgentInvocation, AgentTurnRequest, InvocationRuntime,
};
use tinyagents_harness::store::StoreRegistry;
use tinyagents_registry::DiagnosticSeverity;

use crate::agent::harness::tool_result_artifacts::TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE;
use crate::agent::harness::{run_queue::RunQueue, MAX_SPAWN_DEPTH};
use crate::agent::messages::ChatMessage;
use crate::agent::tinyagents::harness_assembly::{assemble_turn_harness, AssembledTurnHarness};
use crate::agent::tinyagents::host::steering::shared_steering_registry;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::middleware::TurnContextMiddleware;
use crate::agent::tinyagents::observability::{CapPauser, OpenhumanEventBridge, SubagentScope};
use crate::agent::tinyagents::turn_models::TurnModels;
use crate::agent::tinyagents::turn_outcome::TinyagentsTurnOutcome;
use crate::agent::tinyagents::turn_policy::effective_max_iterations;
use crate::agent::tinyagents::turn_run_error::map_turn_run_error;
use crate::agent::tinyagents::turn_run_finalize::finalize_turn_outcome;
use crate::agent::tinyagents::{journal, routes, steering_forwarder};
use tinyagents_harness::ids::TaskId;

use super::ToolPolicyEnforcement;

/// The durable root entry point for hosted turns.  It intentionally carries no
/// product authority: models, tools, middleware, progress and host capabilities
/// are attached through an [`InvocationRuntime`] and [`AgentInvocation`] for
/// each root.  That prevents concurrent sessions from replacing one another's
/// security or workspace state on a shared harness.
static ROOT_HOSTED_HARNESS: LazyLock<AgentHarness<(), OpenHumanRunContext>> =
    LazyLock::new(AgentHarness::new);

fn root_hosted_harness() -> &'static AgentHarness<(), OpenHumanRunContext> {
    &ROOT_HOSTED_HARNESS
}

#[cfg(test)]
#[path = "turn_runner_tests.rs"]
mod tests;

/// Keeps the session's already-built system/context ladder authoritative while
/// still entering TinyAgents through its hosted invocation boundary.  The
/// session request contains the frozen system prompt, prompt policy boundary,
/// context additions and provider-ready history; composing another prompt here
/// would duplicate it and move the cache prefix.
struct PrecomposedRootContext;

#[async_trait]
impl ContextComposer for PrecomposedRootContext {
    async fn compose_system_prompt(
        &self,
        _request: &TurnContextRequest,
    ) -> tinyagents_harness::Result<String> {
        Ok(String::new())
    }

    async fn preamble(
        &self,
        _request: &TurnContextRequest,
    ) -> tinyagents_harness::Result<Vec<tinyinference_llm::message::Message>> {
        Ok(Vec::new())
    }
}

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
    resolved_tools: Vec<Arc<dyn tinytools::Tool>>,
    max_iterations: usize,
) -> Result<TinyagentsTurnOutcome> {
    // `0` means "unset" → the legacy default; otherwise the harness cap would be
    // zero and the run would abort before the first model call.
    let max_iterations = effective_max_iterations(max_iterations);
    let mut harness: tinyagents_harness::runtime::AgentHarness<()> =
        tinyagents_harness::runtime::AgentHarness::new();
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
        harness.register_tool(tool);
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

    let input = crate::agent::message_convert::history_to_messages(&history);
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
    let out_history = crate::agent::message_convert::messages_to_history(&run.messages);
    let conversation = crate::agent::message_convert::messages_to_conversation(
        crate::agent::message_convert::messages_since_request(&run.messages, request_base_len),
    );
    tracing::debug!(
        request_base_len,
        transcript_len = run.messages.len(),
        persisted_messages = run.messages.len().saturating_sub(request_base_len),
        "[tinyagents] persisting post-request transcript (thin path; steer-safe boundary)"
    );

    Ok(TinyagentsTurnOutcome {
        text,
        resolved_route: None,
        history: out_history,
        conversation,
        model_calls: run.model_calls,
        tool_calls: run.tool_calls,
        input_tokens: run.usage.usage.input_tokens,
        output_tokens: run.usage.usage.output_tokens,
        cached_input_tokens: run.usage.usage.cache_read_tokens,
        charged_amount_usd: crate::platform::cost::catalog::estimate_cost_usd(
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
/// in-house `live` turn machine: it registers a canonical shared-tool adapter
/// per advertised spec so the same `Arc`-shared tools the legacy loop runs are
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
/// [`ImageAwareMessageTrimMiddleware`](super::middleware::ImageAwareMessageTrimMiddleware)
/// keeps history under budget (autocompaction parity).
///
/// `run_queue` forwards mid-flight steer messages into the run; `subagent_scope`
/// re-scopes progress to the `Subagent*` variants (child runs); `early_exit_tools`
/// name the tools that pause the loop (e.g. `ask_user_clarification`) and surface
/// the question via [`TinyagentsTurnOutcome::early_exit_tool`].
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_turn_via_tinyagents_shared(
    run_context: OpenHumanRunContext,
    turn_models: TurnModels,
    provider_id: String,
    model: &str,
    history: Vec<ChatMessage>,
    tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    allowed: Option<HashSet<String>>,
    max_iterations: usize,
    subagent_scope: Option<SubagentScope>,
    context_window: Option<u64>,
    run_queue: Option<Arc<RunQueue>>,
    early_exit_tools: &[&str],
    pause_at_cap: bool,
    max_output_tokens: Option<u32>,
    context_mw: TurnContextMiddleware,
    tool_policy: Option<ToolPolicyEnforcement>,
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
    run_turn_via_tinyagents_inner(
        run_context,
        turn_models,
        provider_id,
        model,
        history,
        tool_sets,
        allowed,
        max_iterations,
        subagent_scope,
        context_window,
        run_queue,
        early_exit_tools,
        pause_at_cap,
        max_output_tokens,
        context_mw,
        tool_policy,
        deterministic_cacheable,
        defer_turn_completed_to_caller,
        None,
    )
    .await
}

/// Hosted root-turn entry point.
///
/// Channel and sub-agent callers deliberately remain on
/// [`run_turn_via_tinyagents_shared`] until their own cutovers.  A root cannot
/// reach that legacy entry: it supplies durable host authority here and this
/// function drives the process-shared harness through `AgentInvocation`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_root_turn_via_hosted_agent(
    run_context: OpenHumanRunContext,
    hosted_base: Arc<crate::agent::tinyagents::host::OpenHumanHostBase>,
    agent_id: String,
    turn_models: TurnModels,
    provider_id: String,
    model: &str,
    history: Vec<ChatMessage>,
    tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    allowed: Option<HashSet<String>>,
    max_iterations: usize,
    context_window: Option<u64>,
    run_queue: Option<Arc<RunQueue>>,
    early_exit_tools: &[&str],
    pause_at_cap: bool,
    max_output_tokens: Option<u32>,
    context_mw: TurnContextMiddleware,
    tool_policy: Option<ToolPolicyEnforcement>,
    defer_turn_completed_to_caller: bool,
) -> Result<TinyagentsTurnOutcome> {
    run_turn_via_tinyagents_inner(
        run_context,
        turn_models,
        provider_id,
        model,
        history,
        tool_sets,
        allowed,
        max_iterations,
        None,
        context_window,
        run_queue,
        early_exit_tools,
        pause_at_cap,
        max_output_tokens,
        context_mw,
        tool_policy,
        false,
        defer_turn_completed_to_caller,
        Some((hosted_base, agent_id)),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
#[path = "turn_runner_inner.rs"]
mod turn_runner_inner;
use turn_runner_inner::run_turn_via_tinyagents_inner;
