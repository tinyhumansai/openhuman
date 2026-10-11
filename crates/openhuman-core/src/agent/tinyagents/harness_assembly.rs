//! [`assemble_turn_harness`] — register the provider model, every shared
//! tool, and the full middleware stack in the intended order for one turn.

use std::collections::HashSet;
use std::sync::Arc;

use tinyagents_harness::cache::InMemoryResponseCache;
use tinyagents_harness::middleware::{
    plan_mode_middleware, ApprovalGateMiddleware, ArgRecoveryMiddleware, BudgetLimits,
    BudgetMiddleware, ContextCompressionMiddleware, PromptCacheGuardMiddleware,
    RepeatProgressMiddleware, RunModeHandle, ToolPolicyMiddleware as TaToolPolicyMiddleware,
};
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::steering::SteeringHandle;
use tinyagents_registry::{CapabilityRegistry, RegistryDiagnostic, RegistrySnapshot};
use tinyinference_llm::model::CapabilitySet;
use tokio::sync::mpsc::Sender;

use crate::agent::progress::AgentProgress;
use crate::agent::tinyagents::harness_context_ladder::install_context_ladder;
use crate::agent::tinyagents::harness_tool_registration::register_turn_tools_and_agents;
use crate::agent::tinyagents::host::steering;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::middleware::{self, TurnContextMiddleware};
use crate::agent::tinyagents::model::{ErrorSlotModel, TurnChatModel};
use crate::agent::tinyagents::observability::{
    IterationCursor, ProviderUsageCarry, SubagentScope, ToolFailureMap, ToolNameMap,
};
use crate::agent::tinyagents::routes;
use crate::agent::tinyagents::stop_hooks;
use crate::agent::tinyagents::tools::EarlyExitHook;
use crate::agent::tinyagents::turn_models::TurnModels;
use crate::agent::tinyagents::turn_outcome::{HaltSummarySlot, ToolOutcomeSink};
use crate::agent::tinyagents::turn_policy::{
    run_policy_for_provider, REPEATED_TOOL_FAILURE_THRESHOLD,
};
use crate::agent::tinyagents::verify_before_finish;
use tinyagents_harness::store::InMemoryStore as ToolResultArtifactIndexStore;

use super::ToolPolicyEnforcement;
mod assembled;
pub(super) use assembled::AssembledTurnHarness;

/// Assemble the turn harness for [`run_turn_via_tinyagents_shared`](super::run_turn_via_tinyagents_shared):
/// register the provider model, every shared tool, and the full middleware
/// stack in the intended order. Split out of the runner so the adapter
/// inventory is directly testable (issue #4249, Phase 11) — the returned
/// [`AssembledTurnHarness`] exposes the harness registries without driving a
/// run.
#[allow(clippy::too_many_arguments)]
pub(super) fn assemble_turn_harness(
    turn_models: TurnModels,
    model: &str,
    tool_sets: Vec<Arc<Vec<Box<dyn tinytools::Tool>>>>,
    allowed: Option<HashSet<String>>,
    max_iterations: usize,
    _on_progress: Option<Sender<AgentProgress>>,
    subagent_scope: Option<SubagentScope>,
    context_window: Option<u64>,
    early_exit_tools: &[&str],
    context_mw: TurnContextMiddleware,
    stop_hooks: Vec<
        Arc<dyn tinyagents_harness::middleware::StopHook<crate::agent::cost::TurnCost>>,
    >,
    tool_policy: Option<ToolPolicyEnforcement>,
    required_capabilities: Option<CapabilitySet>,
    deterministic_cacheable: bool,
    // Hosted roots authorize tools through `OpenHumanSecurityGate`; installing
    // the legacy approval middleware as well would issue a second approval.
    hosted_security_gate: bool,
    // Whether this run pauses gracefully at its model-call cap (issue #6014).
    // Only such a run gets the in-loop conclusion: a run that errors at its cap
    // instead (the channel/CLI path, which maps the stop to
    // `MaxIterationsExceeded`) must keep doing that, and handing it a wrap-up
    // would silently convert a documented error into an answer.
    pause_at_cap: bool,
    // The dialect the session composed its prompt for; see
    // `OpenHumanRunContext::tool_dialect`.
    tool_dialect: tinyagents_harness::config::ToolDispatcher,
    // Session-deferred tools (`OpenHumanRunContext::deferred_tool_names`):
    // registered as `Deferred` whatever their own exposure says.
    session_deferred: Arc<HashSet<String>>,
    // Live per-thread Plan/Build mode handle (`agent::tinyagents::run_mode`).
    // `Some` installs `PlanModeMiddleware`, which hides/denies side-effecting
    // tools while the thread is in `RunMode::Plan` — flipped without
    // restarting the run by `plan_exit` or the `agent.set_run_mode` RPC.
    // `None` for a caller with no thread identity (a sub-agent child, most
    // notably), which never runs in plan mode.
    run_mode: Option<RunModeHandle>,
    // The turn's reasoning choice (`reasoning::turn_reasoning_for`), attached
    // to every model request this harness builds.
    reasoning: Option<tinyinference_llm::model::ReasoningConfig>,
    // Whether this turn runs on a chat thread (`OpenHumanRunContext::thread_id`
    // is `Some`). The per-thread `goal_*` tools resolve their target from the
    // run's thread, so a thread-less turn (a headless `inference_agent_chat`
    // without a `thread_id`) is not offered them at all (issue #6956).
    has_thread: bool,
    // This turn's memory binding (`OpenHumanRunContext::memory_turn`): the
    // compaction summarizer recalls under it. `None` leaves compaction
    // memory-free.
    memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
    tool_rules: Option<Arc<tinyagents_harness::tool::ToolRulePolicy>>, // `OpenHumanRunContext::tool_rules`
) -> AssembledTurnHarness {
    let mut harness: AgentHarness<(), OpenHumanRunContext> = AgentHarness::new();
    // Cross-route fallback ownership (issue #4249, Workstream 02.2): populate the
    // SDK `RunPolicy.fallback` with the ordered same-family route chain for this
    // turn's primary model so the harness fails over to a sibling workload tier
    // (e.g. hint:chat → hint:burst) when the primary route errors. Retry stays pinned
    // to a single attempt (see `run_policy_for`) — fallback and retry are
    // independent knobs, and only fallback is enabled here because `ReliableProvider`
    // (still wrapped) does not fail over across the registered tier routes.
    let mut policy = run_policy_for_provider(
        max_iterations,
        deterministic_cacheable,
        turn_models.is_local(),
    );
    let route_fallback = routes::route_fallback_policy(model);
    policy.fallback = route_fallback.clone();
    // Tool discovery: the harness advertises its `tool_search` bridge over the
    // `Deferred` registrations the allowlist admits, ranked by whatever the
    // process installed (`agent::tinyagents::discovery`).
    policy.discovery = super::discovery::discovery_policy();

    policy.tool_dialect = tool_dialect;
    policy.default_reasoning = reasoning;
    // The session composes its prompt for this same dialect: `ToolsSection`
    // renders the protocol block and the catalogue of the visible tools into
    // the system prompt (inside the cacheable prefix, counted by
    // `prompt-size`). Without this the harness appended a second copy of
    // both on every text-dialect call.
    policy.host_renders_tool_catalogue = true;
    crate::tools::rules::install_turn_rules(&mut policy, tool_rules);
    tracing::debug!(
        model,
        ?tool_dialect,
        "[models] turn harness tool dialect pinned from the session"
    );
    tracing::debug!(
        model,
        fallback_chain = ?route_fallback.as_ref().map(|f| &f.models),
        "[models] assembling turn harness with SDK retry/fallback policy"
    );
    harness.with_policy(policy);
    crate::tools::timeout::install_harness_tool_timeouts(&mut harness);
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
    // Shared FIFO carry of per-call provider `BilledUsage`: `UsageCarryMiddleware`
    // pushes each response's usage (charged USD + context window +
    // cache-creation/reasoning tokens, read off the response via G1), the event
    // bridge pops it when recording that call's usage (#4467, item 1). The carry
    // is produced by a wrap-model middleware now, not the adapter, so route models
    // carry no usage side-channel (Phase 5).
    let provider_usage_carry: ProviderUsageCarry = Arc::default();
    // The turn's models are pre-built by `build_turn_models` (the single
    // `native model adapter` construction site) and handed in as crate `ChatModel`s —
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
    // A non-hosted run owns its registry, so its models record into the run's
    // error slot (#6724). A hosted root shares its runtime registry with its
    // sub-agents; there only the resolver's lead model records.
    let slotted = |model: TurnChatModel| -> TurnChatModel {
        if hosted_security_gate {
            model
        } else {
            Arc::new(ErrorSlotModel::new(model, error_slot.clone()))
        }
    };
    let primary = slotted(primary);
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
        let route_model = slotted(route_model);
        capability_registry.replace_model(name.as_str(), route_model.clone());
        harness.register_model(name, route_model);
    }

    // Cost usage capture (issue #4249, Phase 5): feed the event bridge's usage
    // carry from a wrap-model middleware that reads the full `BilledUsage` off each
    // response, instead of every `native model adapter` pushing it. Installed
    // unconditionally — usage flows on every turn — and shares the same carry the
    // bridge drains on `UsageRecorded`.
    harness.push_model_middleware(Arc::new(routes::UsageCarryMiddleware::new(
        provider_usage_carry.clone(),
    )));

    // Route audit capture is a typed run-context concern. The canonical
    // TinyInference decorator attached in `turn_models` stamps the successful
    // unary response and the streamed terminal response, and this middleware
    // moves that metadata into the explicit OpenHuman carrier.
    harness.push_model_middleware(Arc::new(routes::ResolvedRouteMiddleware));

    // Per-call capability gate (issue #4249, Workstream 02.1): when the turn has
    // derivable capability needs (today: vision for a `hint:vision` turn), stamp
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
    let compaction = context_mw.compaction;
    // Captured for the same reason `autocompact_enabled` is — `install` consumes
    // `context_mw` — and used to site microcompact below, after compression.
    let microcompact_keep_recent = context_mw.microcompact_keep_recent;
    let tool_result_artifact_index = context_mw
        .artifact_store
        .as_ref()
        .map(|_| Arc::new(ToolResultArtifactIndexStore::new()));

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
        steering::SteeringRunClass::Background
    } else {
        steering::SteeringRunClass::Interactive
    };
    let handle = Some(steering::openhuman_steering_handle(steering_run_class));

    // Shared by the two breakers below: whichever halts writes the root cause here.
    let halt_summary: HaltSummarySlot = std::sync::Arc::new(std::sync::Mutex::new(None));

    // Repeat-progress breaker (#4463, restoring #4088/#4095): the failure
    // breaker resets on success, so it misses successful no-op loops and repeated
    // narration+call batches. This guard catches identical successful calls or
    // outputs, including repeats with other calls between them (#6275). Polling
    // tools (`wait_subagent`) stay exempt.
    //
    // Pushed first / outermost: `after_tool` runs in reverse registration order,
    // so this guard fingerprints a result only after every other middleware
    // (byte cap, summarizer, memory-protocol note) has finished rewriting it,
    // i.e. exactly what the model sees. Registered any later, a result whose
    // visible note changed between calls would count as identical.
    // Classified read-only tools are the guard's read-only set.
    let read_only_tools: HashSet<String> = tool_sets
        .iter()
        .flat_map(|set| set.iter())
        .filter(|t| t.policy().classified && t.policy().side_effects.read_only)
        .map(|t| t.name().to_string())
        .collect();
    let repeat_progress = handle.as_ref().map(|handle| {
        Arc::new(
            RepeatProgressMiddleware::new(
                handle.clone(),
                halt_summary.clone(),
                Arc::new(middleware::is_repeat_call_exempt),
            )
            .with_read_only(Arc::new(move |name: &str| read_only_tools.contains(name)))
            .with_cleared_placeholder(crate::agent::context::CLEARED_PLACEHOLDER),
        )
    });
    // Time awareness (#6953); before the repeat guard so its notes land last.
    let shell_turn_budget = middleware::install_time_notes(&mut harness);
    if let Some(mw) = &repeat_progress {
        harness.push_middleware(mw.clone());
    }

    // Repeated failures stop the run using each tool's declared side effects.
    let repeated_failure = handle.as_ref().map(|handle| {
        let (t, halt) = (REPEATED_TOOL_FAILURE_THRESHOLD, halt_summary.clone());
        let mw = middleware::RepeatedToolFailureMiddleware::new(handle.clone(), t, halt);
        Arc::new(mw.with_tool_facts(middleware::tool_sets_lookup(tool_sets.clone())))
    });
    if let Some(mw) = &repeated_failure {
        harness.push_middleware(mw.clone());
    }

    stop_hooks::install(
        &mut harness,
        handle.as_ref(),
        model,
        stop_hooks,
        halt_summary.clone(),
    );
    let early_exit_set: HashSet<&str> = early_exit_tools.iter().copied().collect();
    // One hook per run, shared by every early-exit adapter (records the first
    // early-exit and pauses). Requires the steering handle.
    let early_exit_hook = handle
        .as_ref()
        .filter(|_| !early_exit_set.is_empty())
        .map(|h| EarlyExitHook::new(h.clone()));

    let is_subagent_run = subagent_scope.is_some();
    let (tool_count, candidate_names, registry_diagnostics, registry_snapshot) =
        register_turn_tools_and_agents(
            &mut harness,
            &mut capability_registry,
            &tool_sets,
            &allowed,
            &early_exit_set,
            early_exit_hook.as_ref(),
            is_subagent_run,
            has_thread,
            &session_deferred,
        );

    // SHADOW tool-exposure layer (issue #4249, 01.3 — dynamic exposure). Compose
    // the OpenHuman exposure policy as a crate-native selection layer
    // (ToolAllowlistMiddleware + a ContextualToolSelectionMiddleware built via
    // `inheriting`) and run it in shadow: it emits the exposure decision
    // event-native (`AgentEvent::ToolsFiltered`) and logs any divergence between
    // the crate layer's decision and the set OpenHuman actually registered as
    // callable — WITHOUT changing the callable set (byte-identical to today).
    // Folding the remaining host selection policy in `tool_prep.rs` into the
    // middleware is gated on `[tool-exposure]` divergence logs showing parity. Tags encode
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

    // Prompt-cache prefix protection (issue #4249, 03.2). The vendor loop owns
    // the stable-prefix layout (`PromptSegment`s built from the session's frozen
    // system prefix, `frozen_system_prefix_len`); the crate
    // `PromptCacheGuardMiddleware` diffs the cacheable prefix across model calls
    // and records a `CacheLayoutEvent` when volatile content busts it. The guard
    // is returned so the run loop can drain its events into the observability
    // bridge.
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
    let summary_focus_tools = harness
        .tools()
        .schemas()
        .into_iter()
        .chain(harness.tools().deferred_schemas())
        .filter(|schema| {
            crate::inference::tokenjuice::focus::declares_summary_focus(&schema.parameters)
        })
        .map(|schema| schema.name)
        .collect();
    context_mw.install(&mut harness, tool_policies, summary_focus_tools);

    // Plan mode (issue: plan-mode approvals). `run_mode` is `Some` only for a
    // turn with a thread identity (chat, not a sub-agent child); the
    // middleware itself is a no-op whenever the live handle reads
    // `RunMode::Build`, so pushing it unconditionally for those turns is
    // cheap and lets a mid-run `plan_exit`/`agent.set_run_mode` flip take
    // effect on the very next tool exposure or execution check. `.allow(..)`
    // keeps plan-mode-specific and session-bookkeeping tools reachable while
    // planning even though they are not (or should not be gated as)
    // side-effect-free: `plan_exit` (the hand-off signal itself),
    // `request_plan_review` (the review gate IS the consent surface), the
    // session `todo` list, and the per-thread `goal_*` tools.
    if let Some(mode) = run_mode {
        harness.push_middleware(Arc::new(
            plan_mode_middleware(mode, harness.tools().policies()).allow([
                "plan_exit",
                "request_plan_review",
                "todo",
                "goal_set",
                "goal_get",
                "goal_complete",
            ]),
        ));
    }

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
    // `OpenHumanRunContext` explicit carrier owns the per-turn roll-up (deletion
    // ledger row: "crate-internal CostBudgetMiddleware + explicit run context
    // task-local", `docs/tinyagents-full-migration-plan/99-deletion-ledger.md`):
    //   1. ≥ 500 production turns across BOTH parent and sub-agent runs with
    //      ZERO `[budget_shadow]` divergence log lines — proving the crate
    //      tracker's per-run token accounting matches the authoritative runtime
    //      `AgentRun.usage` on every model call.
    //   2. A pricing table wired via `BudgetMiddleware::with_pricing(..)` at
    //      parity with `tinyagents_registry::pricing::estimate_cost_usd`, so the crate can own
    //      MONEY (USD) budgets. Today the shadow compares TOKENS only (the
    //      observe-only crate middleware has no pricing, so its cost stays $0)
    //      and the local gate is the sole money-budget authority.
    //   3. Run-tree rollup wired: the same shared `BudgetTracker` handed to every
    //      sub-agent harness so a parent budget halts a recursive run pre-spend —
    //      replacing the former task-local parent-turn rollup (06-cost step 3
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
    // The context ladder (compression, microcompact, final-call wrap-up,
    // artifact contents list, trim), registered here so each step keeps its
    // place in the middleware order. See `harness_context_ladder.rs`.
    let (compression_mw, wrap_up_fired) = install_context_ladder(
        &mut harness,
        model,
        context_window,
        autocompact_enabled,
        compaction.trigger_tokens,
        compaction.strategy,
        microcompact_keep_recent,
        summarizer_model,
        pause_at_cap && subagent_scope.is_none(),
        &tool_outcome_sink,
        memory_turn,
    );
    let is_subagent = subagent_scope.is_some();
    let agent_id = tool_policy.as_ref().map(|p| p.agent_definition_id.as_str());
    let turn_wall_clock_ms = harness.policy().limits.max_wall_clock_ms;
    verify_before_finish::install(
        &mut harness,
        is_subagent,
        agent_id,
        &wrap_up_fired,
        turn_wall_clock_ms,
    );
    // The rungs above all *tell* the turn to produce its deliverable; this one
    // looks, on the same scope as the requirements check.
    middleware::install_unmet_deliverable(&mut harness, is_subagent, agent_id);

    // Direct web lookup is bounded. Once enough search/fetch results have
    // returned, the web tools leave the request so the run works with what it
    // has; a run with nothing but web tools answers instead. Sub-agent runs
    // keep their own budgets and are not narrowed here.
    if subagent_scope.is_none() {
        harness.push_middleware(Arc::new(middleware::ResearchBudgetMiddleware::new()));
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
    // block that used to live in the legacy tool adapter.
    if !hosted_security_gate {
        harness.push_tool_middleware(Arc::new(ApprovalGateMiddleware::new(
            "approval_security",
            Arc::new(middleware::ApprovalSecurityMiddleware::new(
                tool_sets.clone(),
            )),
        )));
    }

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
    // The packed-tool router below gates on the same session, and `None`
    // disables it, so take a copy before the enforcement is moved.
    let route_session = tool_policy
        .as_ref()
        .map(|enforcement| enforcement.session.clone());
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
    harness.push_tool_middleware(Arc::new(middleware::credential_scrub_middleware()));

    // Malformed-argument recovery (`before_tool`): repair a call's non-object
    // arguments before the crate's schema gate — decode JSON-encoded-string args
    // (optionally markdown-fenced) to an object, or coerce to `{}` only when the
    // tool schema has no required fields (engine parity). A non-object against a
    // required-field schema is left untouched so the crate's
    // `InvalidArgsPolicy::ReturnToolError` admission path reports the original
    // validation error. It never reaches approval/policy wrappers or the tool.
    harness.push_middleware(Arc::new(ArgRecoveryMiddleware::new(tool_sets.clone())));

    // Clamp shell `timeout_secs` to the turn remainder (#6953), on recovered args.
    harness.push_middleware(Arc::new(shell_turn_budget.clamp()));

    // Bare packed-tool routing (`before_tool`, #6276): a call that names a
    // withheld packed tool directly becomes the `use_skill` call that reaches
    // it, ahead of admission, so every gate above still applies. Only when the
    // session lets that tool run, and never without a session. After
    // `ArgRecoveryMiddleware` so it wraps recovered arguments; before the
    // embedder hooks so they observe the call that actually runs.
    let registered_tools = harness.tools().names();
    harness.push_middleware(Arc::new(middleware::PackedToolRouteMiddleware::new(
        registered_tools,
        route_session,
    )));

    // Embedder tool lifecycle hooks. Registered AFTER `ArgRecoveryMiddleware`:
    // `before_tool` runs in registration order, so a hook installed earlier would
    // observe the provider's raw (possibly JSON-encoded-string / non-object)
    // arguments instead of the recovered object that `ToolHookContext` documents.
    // The middleware caches the normalized pre-call arguments by `call_id` and
    // replays them into the `PostToolUse` context. Its `after_tool` is
    // observation-only (never mutates the result), so running first in the
    // reverse-order `after_tool` chain is safe — it cannot perturb the
    // summarization/cap or tool-outcome capture layers.
    let embedder_tool_hooks = crate::agent::hooks::turn_tool_hooks();
    if !embedder_tool_hooks.is_empty() {
        harness.push_middleware(Arc::new(middleware::EmbedderToolHooksMiddleware::new(
            embedder_tool_hooks,
        )));
    }

    // Registered last so its `before_model` sees the request after every
    // reduction step above (compression, microcompact, trim) has run. A tool
    // result they evicted is no longer a repeat the model can see, so the
    // repeat-progress recurrence ledger restarts (#6275).
    if let Some(mw) = &repeat_progress {
        harness.push_middleware(Arc::new(mw.eviction_observer()));
    }
    // Registered after it: the breaker's corrective nudges ride the next
    // request only. Anything earlier (notably the transcript snapshot a failed
    // turn persists) must not see them, or they become durable history (#6725).
    if let Some(mw) = &repeated_failure {
        harness.push_middleware(Arc::new(mw.nudge_injector()));
    }
    // The turn's memory pack rides every request of the turn, after every
    // reduction step and after the transcript snapshot, so it is never
    // compacted, trimmed or persisted (`memory::lifecycle::hooks::pre_turn`).
    harness.push_middleware(Arc::new(middleware::MemoryPackMiddleware));

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
        wrap_up_fired,
        tool_count,
        registry_snapshot,
        registry_diagnostics,
        tool_result_artifact_index,
        compression_mw,
        prompt_cache_guard,
    }
}

#[cfg(test)]
#[path = "harness_assembly_tests.rs"]
mod tests;
