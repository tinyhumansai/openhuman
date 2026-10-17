//! Drive an openhuman agent turn through the `tinyagents` runtime turn driver.
//!
//! [`TurnSpec::run`] is the entry point the channel and sub-agent routes use and
//! [`TurnSpec::run_hosted`] the one for hosted root turns. Both plan the turn
//! with [`OpenHumanTurnPlanner`], let `tinyagents_runtime::turn_driver::TurnDriver` assemble and wire the harness
//! (models, run policy, tools, steering, early exit, the cap pause), then install
//! the host middleware stack ([`turn_middleware`](super::turn_middleware)) and
//! finalize the run into a [`TinyagentsTurnOutcome`].

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use anyhow::Result;
use tinyagents_harness::runtime::{AgentInvocation, AgentTurnRequest, InvocationRuntime};
use tinyagents_harness::store::{InMemoryStore as ToolResultArtifactIndexStore, StoreRegistry};
use tinyagents_harness::turn_policy::effective_max_iterations;
use tinyagents_runtime::turn_driver::{
    SubagentScope as DriverSubagentScope, TurnControls, TurnDriver, TurnScope,
};
use tinyagents_runtime::{DriverRequest, ToolSnapshot};

use crate::agent::harness::tool_result_artifacts::TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware;
use crate::agent::tinyagents::observability::{OpenhumanEventBridge, SubagentScope};
use crate::agent::tinyagents::tool_registration::capability_diagnostics;
use crate::agent::tinyagents::turn_middleware::{
    install_turn_middleware, TurnMiddlewareInputs, TurnWiring,
};
use crate::agent::tinyagents::turn_models::TurnModels;
use crate::agent::tinyagents::turn_outcome::TinyagentsTurnOutcome;
use crate::agent::tinyagents::turn_plan::{
    enforce_registry_diagnostics, turn_models_for_plan, turn_run_policy, OpenHumanTurnPlanner,
};
use crate::agent::tinyagents::turn_run_error::map_turn_run_error;
use crate::agent::tinyagents::turn_run_finalize::finalize_turn_outcome;
use crate::agent::tinyagents::{journal, routes, steering_forwarder};
use tinyagents_harness::events::EventSink;
use tinyagents_harness::observability::turn_observer;
use tinyagents_harness::run_queue::RunQueue;
use tinyagents_harness::turn_outcome::ToolOutcomeSink;
use tinyagents_session::transcript::TranscriptMessage;

use super::ToolPolicyEnforcement;

/// The durable root entry point for hosted turns.  It intentionally carries no
/// product authority: models, tools, middleware, progress and host capabilities
/// are attached through an [`InvocationRuntime`] and [`AgentInvocation`] for
/// each root.  That prevents concurrent sessions from replacing one another's
/// security or workspace state on a shared harness.
#[path = "turn_exec_hosted.rs"]
mod turn_exec_hosted;
use turn_exec_hosted::{root_hosted_harness, PrecomposedRootContext};

#[cfg(test)]
#[path = "turn_exec_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "budget_stop_turn_tests.rs"]
mod budget_stop_tests;

/// Everything one tinyagents turn needs, named.
///
/// Replaces the two positional entry functions this module used to export (one
/// took twenty arguments, two of them adjacent `bool`s): a call site now names
/// each field, so a reordering is visible in the expression.
///
/// `tool_sets` are the routes' **shared**, `Arc`-owned tool registry sets,
/// consulted in order. `allowed` is the callable tool-name whitelist with
/// **fail-closed** semantics (issue #4452): `None` means "no filter supplied"
/// so every tool visible in `tool_sets` is registered; `Some(set)` registers
/// *exactly* the named tools, so `Some(empty)` is an explicit deny-all. That
/// stops a tool-less sub-agent from inheriting the parent's full tool surface.
///
/// `run_queue` forwards mid-flight steer messages into the run;
/// `subagent_scope` re-scopes progress to the `Subagent*` variants (child
/// runs); `early_exit_tools` name the tools that pause the loop (for example
/// `ask_user_clarification`) and surface the question via
/// [`TinyagentsTurnOutcome::early_exit_tool`]. `max_iterations == 0` means
/// "unset". When `context_window` is known the context middleware keeps history
/// under budget.
///
/// `defer_turn_completed_to_caller` (#4457): when `true`, the turn does not emit
/// the terminal `TurnCompleted`; the caller emits it after its post-run wrap-up
/// (the chat path streams a cap checkpoint first).
pub(crate) struct TurnSpec<'a> {
    pub run_context: OpenHumanRunContext,
    pub turn_models: TurnModels,
    pub provider_id: String,
    pub model: &'a str,
    pub history: Vec<TranscriptMessage>,
    pub tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    pub allowed: Option<HashSet<String>>,
    pub max_iterations: usize,
    pub subagent_scope: Option<SubagentScope>,
    pub context_window: Option<u64>,
    pub run_queue: Option<Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    pub early_exit_tools: &'a [&'a str],
    pub pause_at_cap: bool,
    pub max_output_tokens: Option<u32>,
    pub context_mw: TurnContextMiddleware,
    pub tool_policy: Option<ToolPolicyEnforcement>,
    pub deterministic_cacheable: bool,
    pub defer_turn_completed_to_caller: bool,
}

impl TurnSpec<'_> {
    /// Plan the turn with [`OpenHumanTurnPlanner`] and drive it on the runtime
    /// `TurnDriver` (channel/CLI and sub-agent routes).
    pub(crate) async fn run(self) -> Result<TinyagentsTurnOutcome> {
        Box::pin(drive_turn(self, None)).await
    }

    /// Drive a hosted root turn: the same plan, executed through
    /// `AgentInvocation` with durable host authority attached for each root so
    /// concurrent sessions cannot replace one another's security or workspace
    /// state on a shared harness. A root is never a sub-agent and never serves
    /// a cached response, so those two fields are ignored.
    pub(crate) async fn run_hosted(
        mut self,
        hosted_base: Arc<crate::agent::tinyagents::host::OpenHumanHostBase>,
        agent_id: String,
    ) -> Result<TinyagentsTurnOutcome> {
        self.subagent_scope = None;
        self.deterministic_cacheable = false;
        Box::pin(drive_turn(self, Some((hosted_base, agent_id)))).await
    }
}

async fn drive_turn(
    spec: TurnSpec<'_>,
    hosted_root: Option<(
        Arc<crate::agent::tinyagents::host::OpenHumanHostBase>,
        String,
    )>,
) -> Result<TinyagentsTurnOutcome> {
    let TurnSpec {
        mut run_context,
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
    } = spec;
    // The host context is the sole turn carrier. Pulling the sender into this
    // local keeps the event bridge API compact without creating a second
    // parameter path for progress.
    let on_progress = run_context.progress.clone();
    // `0` means "unset" → the legacy default (a native-bus / test convention);
    // otherwise the harness model-call cap would be zero and abort the run before
    // the first provider call.
    let max_iterations = effective_max_iterations(max_iterations);
    let hosted_security_gate = hosted_root.is_some();
    let is_local = turn_models.is_local();
    // Wrap concrete routes before the invocation-local hosted resolver captures them.
    let turn_models = turn_models.with_budget(run_context.model_budget.as_ref());
    let hosted_model_resolver = hosted_root.as_ref().map(|_| {
        Arc::new(
            crate::agent::tinyagents::turn_models::TurnModelResolver::from_turn_models(
                &turn_models,
            ),
        ) as Arc<dyn tinyagents_harness::host::ModelResolver<()>>
    });
    // The host security adapter must see the exact same `Arc`-shared instances
    // the registration resolves against, so retain only the cheap Arc clone for
    // a hosted invocation (never clone the tools themselves).
    let hosted_tool_sets = hosted_root.as_ref().map(|_| tool_sets.clone());
    // Retained for the event bridge (cheap `Arc` clones — never the tools
    // themselves) so it can resolve a live `&dyn Tool` by name and call the
    // tool's OWN `display_label`/`display_detail` instead of only ever
    // guessing from the bare name (issue: tool-call presentation).
    let bridge_tool_sets = tool_sets.clone();
    // A turn runs on a chat thread exactly when its run context carries one;
    // `into_tinyagents` stamps that id onto the `RunConfig` the tools read.
    // Thread-scoped tools (`goal_*`) are only registered when it is `Some`.
    let has_thread = run_context.thread_id.is_some();
    let is_subagent = subagent_scope.is_some();
    let (mut policy, has_route_fallback) = turn_run_policy(
        model,
        max_iterations,
        deterministic_cacheable,
        is_local,
        run_context.tool_dialect,
        crate::agent::tinyagents::reasoning::turn_reasoning_for(
            &run_context,
            hosted_root.as_ref().map(|(base, _)| base.config.as_ref()),
            max_output_tokens,
        ),
        run_context.tool_rules.clone(),
        run_context.tool_call_limit,
    );
    // A `tool_search` of this turn tells its ranker which chat it runs in, so a
    // cloud embedder's disclosure reaches that chat.
    policy.discovery.rank_context =
        crate::agent::tinyagents::discovery::rank_context_for(run_context.approval.chat.as_ref());
    // The turn's crate `ChatModel` set (`turn_models`) and the provider telemetry id are built by
    // the caller via `build_turn_models`: the seam is crate-native and names no `Provider` (#4249,
    // Phase 5). The telemetry id (`{provider_id}.{model}` in Langfuse) rides in as a param.
    let TurnModels {
        primary,
        routes: tier_routes,
        summarizer: summarizer_model,
        error_slot,
        ..
    } = turn_models;
    let plan_models = turn_models_for_plan(
        model,
        primary,
        tier_routes,
        &error_slot,
        hosted_security_gate,
    );
    let planner = OpenHumanTurnPlanner {
        models: plan_models.clone(),
        policy,
        tool_sets: tool_sets.clone(),
        allowed: allowed.clone(),
        has_thread,
        session_deferred: Arc::clone(&run_context.deferred_tool_names),
    };
    let driver = TurnDriver::<(), OpenHumanRunContext>::new(Arc::new(planner), Arc::new(()));

    // Shared slots the middleware writes and the event bridge / finalizer read.
    // They are created here, not by the middleware installer, because the run
    // context (built before the harness is prepared) carries two of them.
    let cursor: tinyagents_harness::events::IterationCursor = Arc::default();
    let tool_names: crate::agent::tinyagents::observability::ToolNameMap = Arc::default();
    let failure_map: crate::agent::tinyagents::observability::ToolFailureMap = Arc::default();
    let provider_usage_carry: crate::agent::tinyagents::observability::ProviderUsageCarry =
        Arc::default();
    let tool_outcome_sink: ToolOutcomeSink = Arc::new(std::sync::Mutex::new(Vec::new()));
    let tool_result_artifact_index = context_mw
        .artifact_store
        .as_ref()
        .map(|_| Arc::new(ToolResultArtifactIndexStore::new()));

    // The hosted graph is the execution boundary of the same root turn the
    // runtime/session already created. Retain that root config id here instead
    // of manufacturing a second literal `agent_turn` root. The driver adds the
    // `scope:root` / `scope:subagent` tag.
    let mut config = run_context
        .root_run_config(if is_subagent {
            "openhuman-subagent"
        } else {
            "openhuman-agent-turn"
        })
        .with_max_model_calls(max_iterations)
        .with_max_tool_calls(run_context.tool_call_limit_for(max_iterations))
        .with_max_depth(super::budget::depth(&run_context))
        .with_tag("openhuman")
        .with_tag(if on_progress.is_some() {
            "observed"
        } else {
            "unobserved"
        });
    // Per-turn output cap rides RunConfig now (Phase 5 groundwork): the loop
    // stamps it onto every `ModelRequest.max_tokens` and the native model adapter
    // adapter honors it, so the cap no longer bakes into the primary + route
    // models. Mirrors the legacy `AGENT_TURN_MAX_OUTPUT_TOKENS` / sub-agent cap.
    if let Some(cap) = max_output_tokens {
        config = config.with_max_turn_output_tokens(cap);
    }

    tracing::info!(
        model,
        max_iterations,
        observed = on_progress.is_some(),
        "[tinyagents] routing turn through the runtime turn driver (shared tools)"
    );

    let input = crate::agent::message_convert::history_to_messages(&history);
    // Explicit persistence boundary (issue #4455): the request transcript length,
    // captured *before* the run consumes `input`. The turn's persisted
    // `conversation` is everything appended past this index — assistant/tool
    // rounds plus any mid-turn steer/collect messages injected as user turns.
    // Anchoring here (instead of the last-user-message suffix) keeps injected
    // steers from moving the boundary and truncating persisted history on both
    // the parent (`session/turn/core.rs`) and subagent (`subagent_host`) paths.
    let request_base_len = input.len();

    // Build the run context: an event sink feeds the progress/cost bridge, the
    // journal and the cap pauser; the driver's steering handle carries
    // mid-flight, early-exit and cap pauses.
    run_context.tool_result_artifact_index = tool_result_artifact_index.clone();
    run_context.tool_outcomes = Some(tool_outcome_sink.clone());
    let mut ctx = run_context.clone().into_tinyagents(config);
    let run_instance_id = ctx.instance_id();
    // Assemble the run's store registry: the tool-result artifact index (when
    // present) and — behind the default-ON session dual-write flag — the
    // session KV store, so the harness carries a handle to the same
    // `{workspace}/tinyagents_store/kv` tree the live dual-write mirrors into
    // (issue #4249, 04.1). Both stores share one registry so neither clobbers
    // the other. Reads stay legacy until 04.2; this registration is additive
    // and best-effort (a workspace-resolve failure just skips it).
    let mut stores: Option<StoreRegistry> = None;
    if let Some(index) = tool_result_artifact_index.clone() {
        stores
            .get_or_insert_with(StoreRegistry::new)
            .register(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE, index);
    }
    // `session_kv_store` self-gates on the dual-write flag (config default ON +
    // env kill switch), returning `None` when disabled or unresolvable.
    if let Some(session_kv) =
        crate::agent::session_import::live::session_kv_store(&run_context.dispatch_ctx()).await
    {
        stores.get_or_insert_with(StoreRegistry::new).register(
            crate::agent::session_import::live::TINYAGENTS_SESSION_KV_STORE,
            session_kv,
        );
        tracing::debug!(
            "[session-store] registered session kv store on RunContext.stores under '{}'",
            crate::agent::session_import::live::TINYAGENTS_SESSION_KV_STORE
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
    let turn_completed_sink = (!is_subagent && !defer_turn_completed_to_caller)
        .then(|| on_progress.clone())
        .flatten();
    // The EventSink is created unconditionally (bridge, cap pauser and the
    // durable journal of issue #4249, 05.1 all subscribe; it is inert when
    // nothing does), so an unobserved turn stays reconstructable.
    //
    // Mint the durable run id *before* the sink and seed the sink stream prefix
    // with it (`with_stream_id`), so every persisted observation's `event_id` is
    // the restart-stable `{run_id}-evt-{offset}` a late-attach replay
    // reconstructs the timeline from (05.1). The same id keys the journal + status.
    let journal_run_id = tinyagents_harness::observability::mint_run_id();
    if let Some(scope) = &subagent_scope {
        if let Some(slot) = &scope.journal_run_id {
            *slot
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(journal_run_id.as_str().to_string());
        }
    }
    let sink = EventSink::with_stream_id(journal_run_id.as_str());
    // Attach the event bridge for EVERY turn — including an unobserved
    // (`on_progress = None`) background/cron turn (#4467, item 3). The bridge's
    // `record_usage` feeds the global cost tracker on each `UsageRecorded` event
    // *during* the run, so a run that burns N model calls and then fails still
    // contributes that spend to the wallet/cost surfaces — the post-run
    // `record_unobserved_turn_usage` fallback only runs on the success path
    // and never sees a failed run's usage. With `on_progress = None` the bridge
    // still records cost but its progress `send`s are inert no-ops, so there is
    // no spurious streaming; every turn has an event sink.
    let bridge = {
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
            bridge_tool_sets.clone(),
            run_context.dispatch_ctx(),
        );
        bridge.set_definition(
            run_context
                .memory_identity
                .as_ref()
                .and_then(|identity| identity.agent_id.as_deref()),
        );
        sink.subscribe(bridge.clone());
        bridge
    };

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
        .map(|scope| tinyagents_harness::ids::ThreadId::new(scope.task_id.clone()));
    let turn_journal = journal::attach_turn_journal(
        &run_context.dispatch_ctx(),
        &sink,
        model,
        journal_run_id.clone(),
        journal_thread_id,
    )
    .await;
    if !is_subagent {
        if let Some(crate::agent::turn_origin::AgentTurnOrigin::WebChat {
            request_id: Some(request_id),
            ..
        }) = run_context.origin.as_ref()
        {
            journal::register_request_journal_run(
                &run_context.dispatch_ctx(),
                request_id,
                journal_run_id.as_str(),
            );
        }
    }
    ctx = ctx.with_events(sink.clone());

    // Per-turn controls for the driver. Best-effort thread label for the
    // delivery/requeue observability events and the metadata on any requeued
    // steer: a sub-agent uses its task id; the interactive/channel parent turn
    // reads the turn origin.
    let steer_thread_label =
        steering_forwarder::thread_label(subagent_scope.as_ref(), run_context.origin.as_ref());
    let controls = TurnControls {
        max_iterations: Some(max_iterations),
        pause_at_cap,
        early_exit_tools: early_exit_tools
            .iter()
            .map(|name| (*name).to_string())
            .collect::<BTreeSet<_>>(),
        // The host emits the terminal `TurnCompleted` itself (it carries the
        // stop classification), so the driver never fires a completion sink.
        defer_turn_completed: true,
        scope: match &subagent_scope {
            Some(scope) => TurnScope::Subagent(DriverSubagentScope {
                agent_id: scope.agent_id.clone(),
                task_id: scope.task_id.clone(),
            }),
            None => TurnScope::Root,
        },
        // Forwards mid-flight steers from the session's run queue, drains
        // residual (delivered-but-unapplied) steers back into it when the turn
        // ends — also on cancellation (issue #4456).
        steering: run_queue.map(steering_forwarder::queue_feed),
        registrar: is_subagent
            .then(|| Arc::new(steering_forwarder::SharedSteeringRegistrar) as Arc<_>),
        thread_label: Some(steer_thread_label),
        // Only the top-level turn's cap pause is binding on dispatch; a
        // delegated child reaching its own cap is routine and must not stop the
        // parent's fan-out (it still gets its advisory `Pause`). `Pause` is
        // advisory, so the pause is also recorded on the turn's dispatch guard
        // to stop a racing sub-agent dispatch (#5804).
        cap_pause_recorder: (!is_subagent)
            .then(|| run_context.dispatch.clone())
            .flatten()
            .map(|guard| {
                Arc::new(move |completed: u64, cap: u64| {
                    guard.record_pause_requested(completed, cap)
                }) as tinyagents_harness::events::PauseRecorder
            }),
        ..TurnControls::default()
    };
    let request = DriverRequest {
        history: input,
        // A turn driven outside a `Session` has no frozen declaration set; the
        // planner disables the snapshot comparison.
        tools: ToolSnapshot::new(Vec::new()).map_err(|error| anyhow::anyhow!("{error}"))?,
        run_context: ctx,
        stream: streaming,
        controls,
    };
    let mut prepared = driver.prepare(request).await.map_err(|failure| {
        tracing::error!(error = %failure.error, "[tinyagents] turn driver could not prepare the turn");
        anyhow::anyhow!("{}", failure.error)
    })?;
    let steering =
        prepared.run_context.steering.clone().ok_or_else(|| {
            anyhow::anyhow!("turn driver prepared a run without a steering handle")
        })?;
    let tool_count = prepared.tools().registered.len();
    // A turn without the `todo` tool (tools suppressed, or `todo` not admitted)
    // cannot tick its plan, so it gets no plan-order middleware.
    let todo_registered = prepared
        .tools()
        .registered
        .iter()
        .any(|name| name == "todo");

    let candidate_names = prepared.tools().plan.candidates.clone();
    let TurnWiring {
        halt_summary,
        wrap_up_fired,
        compression_mw,
        prompt_cache_guard,
    } = install_turn_middleware(
        &mut prepared.harness,
        &steering,
        TurnMiddlewareInputs {
            model: model.to_string(),
            tool_sets,
            allowed,
            candidate_names,
            subagent_scope: subagent_scope.clone(),
            context_window,
            context_mw,
            host_policy: run_context.harness_host_policy(),
            tool_policy: tool_policy.clone(),
            required_capabilities: routes::turn_required_capabilities(model),
            deterministic_cacheable,
            has_route_fallback,
            dispatch: run_context.dispatch_ctx(),
            hosted_security_gate,
            pause_at_cap,
            run_mode: run_context.thread_id.as_deref().map(|thread_id| {
                crate::agent::tinyagents::run_mode::handle_for_thread(
                    &run_context.dispatch_ctx(),
                    thread_id,
                )
            }),
            memory_turn: run_context.memory_turn.clone(),
            summarizer_model,
            failure_map: failure_map.clone(),
            provider_usage_carry: provider_usage_carry.clone(),
            tool_outcome_sink: tool_outcome_sink.clone(),
            // The store the `todo` tool resolves from the same parent context.
            todo_store: run_context
                .parent
                .as_ref()
                .filter(|_| todo_registered)
                .map(|parent| crate::agent::todos::ops::store(&parent.workspace_dir)),
        },
    );
    if let Some(scope) = run_context.response_shape.clone() {
        tinyagents_harness::middleware::install_response_shape(&mut prepared.harness, scope);
    }
    super::budget::install_depth(&mut prepared.harness, &run_context);
    super::deadline_wind_down::install(
        &mut prepared.harness,
        &Some(steering.clone()),
        &run_context,
        &subagent_scope,
    );
    turn_observer::install(&mut prepared.harness, &sink, hosted_security_gate);

    // Fail-closed registry validation gate (issue #4249, Workstream 10 — registry).
    // The projected `CapabilityRegistry` produced these diagnostics while the
    // turn was planned; enforce them here, *before* the first model dispatch, so
    // an ambiguous/broken tool surface (duplicate name across native/MCP/Composio/
    // generated tools, dangling alias, etc.) aborts the turn instead of silently
    // resolving to an unintended component while a provider call is in flight.
    let registry_diagnostics = capability_diagnostics(
        &plan_models,
        &bridge_tool_sets,
        &prepared.tools().plan.admitted,
    );
    if let Err(error) = enforce_registry_diagnostics(&registry_diagnostics) {
        // Settle the driver's steering registration before the turn aborts.
        let _ = prepared.split().1.finish(None);
        return Err(error);
    }
    tracing::debug!(
        model,
        tools = tool_count,
        "[tinyagents] turn harness assembled"
    );

    // Heap-allocate the harness drive future. It is large (it owns the whole run
    // context, middleware stack, and loop state), and a sub-agent turn runs
    // nested inside its parent's drive future — leaving it inline on the stack
    // overflows when the parent + child drives compose. Boxing keeps only a
    // pointer on the stack at each level.
    let resolved_route_slot = run_context.resolved_route.clone();
    let (run_result, early_exit) = if let Some((base, agent_id)) = hosted_root {
        let tool_sets = hosted_tool_sets.expect("hosted root retained its tool sets");
        let mut host_bundle =
            crate::agent::tinyagents::host::OpenHumanHostBundleFactory::build_for_invocation(
                crate::agent::tinyagents::host::OpenHumanHostInvocationInputs {
                    base,
                    tool_sets,
                    tool_policy: tool_policy
                        .as_ref()
                        .map(|policy| Arc::new(policy.session.clone())),
                    model_resolver: hosted_model_resolver,
                },
                &run_context,
            );
        // The root session has already assembled its exact system prompt,
        // learned context and history before this graph runs.  Keep those bytes
        // as the hosted request rather than composing/recalling a second copy;
        // the host still owns definition resolution, security, model routing,
        // budgets, progress and outcome classification for this invocation.
        host_bundle.capabilities.context = Arc::new(PrecomposedRootContext);
        host_bundle.capabilities.memory = None;
        host_bundle.capabilities.experience = None;
        // Session finalization owns the full-fidelity hook payload (including
        // sanitized per-tool outcomes).  The generic names-only summary would
        // otherwise fire the same hooks a second time.
        host_bundle.capabilities.learning = None;

        let (parts, tail) = prepared.split();
        let invocation = AgentInvocation::new(
            host_bundle.capabilities,
            AgentTurnRequest::new(agent_id, parts.history),
            parts.run_context,
        )
        .with_replayed_prefix(request_base_len.saturating_sub(1)) // #6710: only the new input is screened
        .with_runtime(InvocationRuntime::new(parts.harness));
        let state = ();
        // Both surfaces return the harness's typed `HostedError`, so a timeout,
        // a limit and a provider failure keep their kind through the turn error
        // and `web_errors` can classify them (#6375). Draining
        // `invoke_agent_stream` instead would collapse every failure to one
        // sanitized string. Streaming still drives every model call through
        // `ChatModel::stream`; this path reads only the terminal run.
        let hosted = root_hosted_harness();
        let outcome = if streaming {
            hosted.invoke_agent_streaming(invocation, &state).await
        } else {
            hosted.invoke_agent(invocation, &state).await
        };
        let outcome = outcome.map_err(|error| {
            tracing::debug!(
                kind = ?error.kind,
                streaming,
                "[tinyagents] hosted root turn failed; keeping the typed kind"
            );
            super::hosted_error::run_error_from_hosted(error)
        });
        // Stop forwarding and deregister now: the poll loop must not deliver
        // into the no-longer-drained handle during post-run journal/mapping
        // work. On a *cancelled* turn this line is never reached; the guard's
        // `Drop` fires as the turn future unwinds, giving identical cleanup.
        let report = tail.finish(outcome.as_ref().ok());
        (outcome, report.early_exit)
    } else {
        let run = Box::pin(prepared.run()).await;
        let early_exit = run.report.early_exit;
        (
            match run.partial.error {
                Some(error) => Err(error),
                None => Ok(run.partial.run),
            },
            early_exit,
        )
    };
    let run = match run_result {
        Ok(run) => run,
        Err(e) => {
            return Err(map_turn_run_error(
                e,
                model,
                max_iterations,
                &error_slot,
                turn_journal.as_ref(),
            )
            .await);
        }
    };

    let resolved_route = resolved_route_slot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    Ok(finalize_turn_outcome(
        run,
        model,
        max_iterations,
        &subagent_scope,
        pause_at_cap,
        turn_journal.as_ref(),
        compression_mw.as_ref(),
        prompt_cache_guard.as_ref(),
        turn_completed_sink,
        Some(bridge),
        early_exit,
        &halt_summary,
        &wrap_up_fired,
        run_instance_id,
        &tool_outcome_sink,
        resolved_route,
        request_base_len,
        &run_context.dispatch_ctx(),
    )
    .await)
}
