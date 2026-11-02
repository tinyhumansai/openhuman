//! The host's per-turn middleware stack, installed on the harness a
//! [`TurnDriver`](tinyagents_runtime::turn_driver::TurnDriver) prepared.
//!
//! The driver owns the generic half of a turn (models, run policy, tool
//! registration, steering, early exit, the cap pause). Everything product-shaped
//! that wraps a model call or a tool call lives here, in the intended order:
//! usage and route capture, time awareness and the repeat/failure breakers,
//! tool exposure, the prompt-cache guard, tool-outcome capture, the context
//! ladder, plan mode, budgets, the tool-boundary gates (approval, CLI/RPC
//! scope, session tool policy, credential scrub) and the final request shaping
//! (argument recovery, embedder hooks, nudges, the memory pack).

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::cache::InMemoryResponseCache;
use tinyagents_harness::host::security_gate::{GateDecision, SecurityGate, ToolCallRequest};
use tinyagents_harness::middleware::{
    install_unmet_deliverable, is_repeat_call_exempt, tool_sets_lookup, ResearchBudgetMiddleware,
    ShellTurnBudget, TurnClockMiddleware, DIRECT_WEB_READ_LIMIT,
};
use tinyagents_harness::middleware::{
    plan_mode_middleware, ApprovalGateMiddleware, ArgRecoveryMiddleware, BudgetLimits,
    BudgetMiddleware, ContextCompressionMiddleware, MiddlewareToolOutcome,
    PromptCacheGuardMiddleware, RepeatProgressMiddleware, RunModeHandle, ToolHandler,
    ToolMiddleware, ToolPolicyMiddleware as TaToolPolicyMiddleware,
};
use tinyagents_harness::runtime::AgentHarness;
use tinyagents_harness::steering::SteeringHandle;
use tinyagents_harness::turn_outcome::{HaltSummarySlot, ToolOutcomeSink};
use tinyagents_harness::turn_policy::REPEATED_TOOL_FAILURE_THRESHOLD;
use tinyagents_hooks::lifecycle::BudgetParityObserver;
use tinyinference_llm::model::CapabilitySet;

use crate::agent::tinyagents::context_wiring::install_context_ladder;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::host::OpenHumanSecurityGate;
use crate::agent::tinyagents::middleware;
use crate::agent::tinyagents::middleware::turn_context::TurnContextMiddleware;
use crate::agent::tinyagents::observability::{ProviderUsageCarry, SubagentScope, ToolFailureMap};
use crate::agent::tinyagents::verify_before_finish;

use super::ToolPolicyEnforcement;
#[path = "turn_native_security.rs"]
mod native_security;
use native_security::NativeSubagentSecurityMiddleware;

struct OpenHumanCostEstimator;

impl tinyagents_harness::middleware::library::CostEstimator for OpenHumanCostEstimator {
    fn estimate(&self, model: &str, usage: &tinyinference_llm::usage::BilledUsage) -> Option<f64> {
        crate::agent::cost::estimate_call_cost_usd(model, usage)
    }
}

/// What the middleware stack needs to know about the turn.
pub(super) struct TurnMiddlewareInputs {
    /// Configured provider id used only when the response has no resolved route.
    pub(super) provider_id: String,
    /// The turn's primary model name.
    pub(super) model: String,
    /// The shared tool sets the turn's tools resolve against.
    pub(super) tool_sets: Vec<tinyagents_runtime::turn_driver::SharedToolSet>,
    /// The allowlist the turn registered under (`None` = every visible tool).
    pub(super) allowed: Option<HashSet<String>>,
    /// The tool names the registration considered (before the allowlist).
    pub(super) candidate_names: Vec<String>,
    /// Set for a delegated child run.
    pub(super) subagent_scope: Option<SubagentScope>,
    pub(super) context_window: Option<u64>,
    pub(super) context_mw: TurnContextMiddleware,
    /// Stop policies and scoped tool callbacks (registered after the runtime's
    /// global ones), from the explicit run context.
    pub(super) host_policy: super::host::HarnessHostPolicy,
    pub(super) tool_policy: Option<ToolPolicyEnforcement>,
    pub(super) required_capabilities: Option<CapabilitySet>,
    pub(super) deterministic_cacheable: bool,
    /// Whether the run has a cross-route fallback chain.
    pub(super) has_route_fallback: bool,
    /// The turn's dispatch context (its embedder tool hooks come from it).
    pub(super) dispatch: crate::core::runtime::DispatchCtx,
    /// Hosted roots authorize tools through `OpenHumanSecurityGate`; installing
    /// the legacy approval middleware as well would issue a second approval.
    pub(super) hosted_security_gate: bool,
    /// Host gate installed at the native child's tool boundary, avoiding a
    /// full hosted root invocation while preserving the same authorization.
    pub(super) native_security_gate: Option<(Arc<OpenHumanSecurityGate>, String)>,
    /// Whether this run pauses gracefully at its model-call cap (issue #6014).
    /// Only such a run gets the in-loop conclusion: a run that errors at its cap
    /// instead (the channel/CLI path, which maps the stop to
    /// `MaxIterationsExceeded`) must keep doing that, and handing it a wrap-up
    /// would silently convert a documented error into an answer.
    pub(super) pause_at_cap: bool,
    /// Live per-thread Plan/Build mode handle; `Some` installs
    /// `PlanModeMiddleware`. `None` for a caller with no thread identity.
    pub(super) run_mode: Option<RunModeHandle>,
    /// This turn's memory binding: the compaction summarizer recalls under it.
    pub(super) memory_turn: Option<Arc<crate::memory::lifecycle::hooks::MemoryTurn>>,
    /// The context-window summarizer: its model, the id it calls
    /// (`[context] summarizer_model`, else the turn's model) and its
    /// rendering and output cap.
    pub(super) summarizer: super::context_wiring::CompactionSummarizer,
    /// Shared `call_id -> (success, failure, elapsed_ms, output_chars)` side
    /// channel: middleware records each outcome for the bridge's
    /// `ToolCallCompleted`.
    pub(super) failure_map: ToolFailureMap,
    /// Shared FIFO carry of per-call provider `BilledUsage`; the event bridge
    /// reads it when recording usage to preserve charged-USD precedence (#4467).
    pub(super) provider_usage_carry: ProviderUsageCarry,
    /// Per-call tool success/content capture for honest `ToolCallRecord`s.
    pub(super) tool_outcome_sink: ToolOutcomeSink,
    /// The store the `todo` tool writes this turn's plan to; `Some` lets the
    /// plan-order middleware read it. `None` for a turn with no workspace or
    /// without the `todo` tool.
    pub(super) todo_store: Option<Arc<dyn tinyagents_harness::store::Store>>,
    /// Called when the turn starts summarizing earlier conversation, so the
    /// client can show it. `None` for a turn nobody watches.
    pub(super) compaction_notice: Option<super::context_wiring::CompactionNotice>,
    /// The session store the goal and `todo` tools write, whether or not this
    /// turn has the `todo` tool; the run-state block reads it.
    /// With the scope the `todo` tool uses on a turn without a thread (the
    /// session id), so a headless turn's block still finds its plan.
    pub(super) state_store: Option<(Arc<dyn tinyagents_harness::store::Store>, String)>,
    /// The turn suppresses the thread goal (`TurnOverrides::suppress_active_goal`).
    pub(super) goal_suppressed: bool,
}

/// The handles the run loop reads after the drive returns.
pub(super) struct TurnWiring {
    /// Root-cause summary recorded by the repeated-tool-failure breaker.
    pub(super) halt_summary: HaltSummarySlot,
    /// Set by `FinalCallWrapUpMiddleware` when it turned the last permitted
    /// model call into the turn's conclusion (issue #6014). `None` when the
    /// middleware is not installed (a run that does not pause at its cap).
    pub(super) wrap_up_fired:
        Option<Arc<tinyagents_harness::middleware::FinalCallWrapUpMiddleware>>,
    /// The installed compression middleware, drained after the run to surface
    /// each compaction's provenance.
    pub(super) compression_mw: Option<Arc<ContextCompressionMiddleware>>,
    /// Records a `CacheLayoutEvent` whenever the cacheable prompt prefix changes
    /// across model calls (issue #4249, 03.2).
    pub(super) prompt_cache_guard: Arc<PromptCacheGuardMiddleware>,
}

/// Installs the host's middleware stack onto `harness`, whose models, run
/// policy and tools the driver already registered. `steering` is the run's own
/// handle (the breakers, stop hooks and nudges pause through it).
pub(super) fn install_turn_middleware(
    harness: &mut AgentHarness<(), OpenHumanRunContext>,
    steering: &SteeringHandle,
    inputs: TurnMiddlewareInputs,
) -> TurnWiring {
    let TurnMiddlewareInputs {
        provider_id,
        model,
        tool_sets,
        allowed,
        candidate_names,
        subagent_scope,
        context_window,
        context_mw,
        host_policy,
        tool_policy,
        required_capabilities,
        deterministic_cacheable,
        has_route_fallback,
        dispatch,
        hosted_security_gate,
        native_security_gate,
        pause_at_cap,
        run_mode,
        memory_turn,
        summarizer,
        failure_map,
        provider_usage_carry,
        tool_outcome_sink,
        todo_store,
        compaction_notice,
        state_store,
        goal_suppressed,
    } = inputs;
    let super::host::HarnessHostPolicy {
        stop_hooks,
        tool_hooks: scoped_tool_hooks,
        ..
    } = host_policy;
    let model = model.as_str();
    crate::tools::timeout::install_harness_tool_timeouts(harness);
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
    // The run's own steering handle: besides run-queue steering, the early-exit /
    // cap / stop-hook pauses, the repeated-tool-failure breaker also pauses
    // through it, and it wants to fire on every path.
    let handle = Some(steering.clone());

    // Cost usage capture (issue #4249, Phase 5): feed the event bridge's usage
    // carry from a wrap-model middleware that reads the full `BilledUsage` off each
    // response, instead of every `native model adapter` pushing it. Installed
    // unconditionally — usage flows on every turn — and shares the same carry the
    // bridge drains on `UsageRecorded`.
    let usage_carry = provider_usage_carry.clone();
    harness.push_model_middleware(Arc::new(
        tinyagents_harness::middleware::library::UsageCarryMiddleware::new(move |response| {
            if let Some(usage) = tinyinference_llm::usage::usage_info_from_response(response) {
                usage_carry
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .push_back(usage);
            }
        }),
    ));

    // Native route enrichment records the canonical resolved route in this
    // run's explicit host carrier; pricing and cost remain host-owned.
    harness.push_model_middleware(Arc::new(
        tinyagents_harness::middleware::library::ResolvedRouteMiddleware::new(
            provider_id,
            model,
            OpenHumanCostEstimator,
            |data: &OpenHumanRunContext, route| {
                *data
                    .resolved_route
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(route);
            },
        )
        .with_pricing_lookup(generation_pricing),
    ));

    // Per-call capability gate (issue #4249, Workstream 02.1): when the turn has
    // derivable capability needs (today: vision for a `hint:vision` turn), stamp
    // them onto every `ModelRequest` via `with_required_capabilities` so an unfit
    // model is rejected pre-dispatch (and, once 02.2 lands, a capable fallback is
    // selected) instead of failing at the provider.
    if let Some(required) = required_capabilities {
        harness.push_model_middleware(Arc::new(
            tinyagents_harness::middleware::library::RequiredCapabilitiesMiddleware::new(required),
        ));
    }

    // Fallback event parity (issue #4249, Workstream 02.2): the crate's
    // registry-backed `RunPolicy.fallback` traversal (wired above) performs the
    // cross-route swap silently — it emits no `AgentEvent::FallbackSelected`. Wrap
    // the model-resolving core with an observer that surfaces the parity event
    // whenever the resolved model differs from the primary, so a fallback is
    // visible on the OpenHuman progress/observability bridge (and grep-logged under
    // `[fallback]`). Installed only when a fallback chain exists; it never re-issues
    // the call, so it adds no provider dispatch (no double-fallback).
    if has_route_fallback {
        harness.push_model_middleware(Arc::new(
            tinyagents_harness::middleware::library::FallbackObserverMiddleware::new(model),
        ));
    }

    // Capture context settings before `install` consumes `context_mw`.
    let autocompact_enabled = context_mw.autocompact_enabled;
    let compaction = context_mw.compaction;
    // Captured for the same reason `autocompact_enabled` is — `install` consumes
    // `context_mw` — and used to site microcompact below, after compression.
    let microcompact_keep_recent = context_mw.microcompact_keep_recent;

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
                Arc::new(is_repeat_call_exempt),
            )
            .with_read_only(Arc::new(move |name: &str| read_only_tools.contains(name)))
            .with_cleared_placeholder(
                tinyagents_harness::middleware::library::DEFAULT_CLEARED_PLACEHOLDER,
            ),
        )
    });
    // Time awareness (#6953); before the repeat guard so its notes land last.
    let turn_budget = harness
        .policy()
        .limits
        .max_wall_clock_ms
        .map(std::time::Duration::from_millis);
    harness.push_middleware(Arc::new(TurnClockMiddleware::new(turn_budget)));
    let shell_turn_budget = ShellTurnBudget::with_max_timeout(
        turn_budget,
        tinyagents_harness_tools::exec::timeout::MAX_TIMEOUT_SECS,
    );
    harness.push_middleware(Arc::new(shell_turn_budget.notes()));
    if let Some(mw) = &repeat_progress {
        harness.push_middleware(mw.clone());
    }

    // Repeated failures stop the run using each tool's declared side effects.
    let repeated_failure = handle.as_ref().map(|handle| {
        let (t, halt) = (REPEATED_TOOL_FAILURE_THRESHOLD, halt_summary.clone());
        let mw = middleware::repeated_failure::RepeatedToolFailureMiddleware::new(
            handle.clone(),
            t,
            halt,
        );
        Arc::new(mw.with_tool_facts(tool_sets_lookup(tool_sets.clone())))
    });
    if let Some(mw) = &repeated_failure {
        harness.push_middleware(mw.clone());
    }

    if handle.is_some() && !stop_hooks.is_empty() {
        let summary = halt_summary.clone();
        harness.push_middleware(Arc::new(
            tinyagents_hooks::stop::StopHookMiddleware::new(model, stop_hooks)
                .with_metrics_seed(crate::agent::cost::turn_cost())
                .with_stop_observer(move |report| {
                    tracing::warn!(
                        target: "stop_hooks",
                        hook = %report.hook,
                        iteration = report.iteration,
                        model = %report.model,
                        "[stop_hooks] hook voted to stop the turn — pausing run: {}",
                        report.reason
                    );
                    record_stop_summary(&summary, report.iteration, &report.reason);
                }),
        ));
    }
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
        tinyagents_harness::middleware::ToolExposureShadowMiddleware::new(
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
    harness.push_middleware(Arc::new(
        middleware::tool_outcome_capture::ToolOutcomeCaptureMiddleware::new(
            tool_outcome_sink.clone(),
            failure_map.clone(),
        ),
    ));

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
    context_mw.install(harness, tool_policies, summary_focus_tools);

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
            plan_mode_middleware(mode, harness.tools().policies()).allow(
                tinyagents_harness::middleware::library::plan_mode_allowlist()
                    .iter()
                    .copied(),
            ),
        ));
    }

    // Observe-only crate `BudgetMiddleware`, installed with empty limits so it
    // never enforces: it only folds each call's usage into a shared tracker.
    // The spend cap is gone; `BudgetParityObserver` below compares that tracker
    // against the authoritative `AgentRun.usage` at end of run and logs
    // `[budget_parity]` parity or divergence.
    let shadow_budget = Arc::new(BudgetMiddleware::new(BudgetLimits::default()));
    let shadow_budget_tracker = shadow_budget.tracker();
    harness.push_middleware(shadow_budget);

    harness.push_middleware(Arc::new(BudgetParityObserver::new(shadow_budget_tracker)));
    // The context ladder (compression, microcompact, final-call wrap-up,
    // artifact contents list, trim), registered here so each step keeps its
    // place in the middleware order. See `context_wiring.rs`.
    let (compression_mw, wrap_up_fired) = install_context_ladder(
        harness,
        model,
        context_window,
        autocompact_enabled,
        compaction.trigger_tokens,
        compaction.strategy,
        microcompact_keep_recent,
        summarizer,
        pause_at_cap && subagent_scope.is_none(),
        &tool_outcome_sink,
        memory_turn,
        compaction_notice,
    );
    let is_subagent = subagent_scope.is_some();
    let agent_id = tool_policy.as_ref().map(|p| p.agent_definition_id.as_str());
    let turn_wall_clock_ms = harness.policy().limits.max_wall_clock_ms;
    verify_before_finish::install(
        harness,
        is_subagent,
        agent_id,
        &wrap_up_fired,
        turn_wall_clock_ms,
    );
    // The rungs above all *tell* the turn to produce its deliverable; this one
    // looks, on the same scope as the requirements check.
    if verify_before_finish::applies(is_subagent, agent_id) {
        install_unmet_deliverable(harness);
    } else {
        tracing::debug!(
            is_subagent,
            agent = agent_id.unwrap_or("<none>"),
            "[unmet_deliverable] not installed for this turn"
        );
    }
    // Plan steps run in order: no work while no step is in progress, a final
    // answer is held once while steps are open, a step stuck in progress gets
    // one reminder. Same scope as the requirements check, and registered after
    // it so its `after_model` runs first and the plan check comes first.
    match todo_store {
        Some(store) if verify_before_finish::applies(is_subagent, agent_id) => {
            // A thread-less turn's plan sits under the session id (the `todo`
            // tool's fallback scope); without the same key these rules never
            // see it.
            let mut sequence = tinyagents_harness_tools::todos::TodoSequenceMiddleware::new(store);
            if let Some((_, fallback_thread)) = &state_store {
                sequence = sequence.with_fallback_thread(fallback_thread.clone());
            }
            harness.push_middleware(Arc::new(sequence));
        }
        _ => tracing::debug!(
            is_subagent,
            agent = agent_id.unwrap_or("<none>"),
            "[todo_sequence] not installed for this turn"
        ),
    }
    // The goal, the plan and the turn's tool-call facts on every model call,
    // request-only. On these turns the goal is not prepended to the user's
    // message (see `state_block`).
    super::state_block::install(harness, state_store, is_subagent, agent_id, goal_suppressed);

    // Direct web lookup is bounded. Once enough search/fetch results have
    // returned, the web tools leave the request so the run works with what it
    // has; a run with nothing but web tools answers instead. Sub-agent runs
    // keep their own budgets and are not narrowed here.
    if subagent_scope.is_none() {
        harness.push_middleware(Arc::new(ResearchBudgetMiddleware::with_limit(
            DIRECT_WEB_READ_LIMIT,
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
    // block that used to live in the legacy tool adapter.
    let native_security_gate_installed = native_security_gate.is_some();
    if let Some((gate, agent_id)) = native_security_gate {
        harness.push_tool_middleware(Arc::new(NativeSubagentSecurityMiddleware::new(
            gate, agent_id,
        )));
    }
    if !hosted_security_gate {
        harness.push_tool_middleware(Arc::new(ApprovalGateMiddleware::new(
            "approval_security",
            Arc::new(middleware::approval::ApprovalSecurityMiddleware::new(
                tool_sets.clone(),
            )),
        )));
    }

    // CLI/RPC-only scope gate — a tool restricted to explicit CLI/RPC invocation
    // must not run from the model loop. Intrinsic to the tool, so installed on
    // every path (channel/session/sub-agent).
    harness.push_tool_middleware(Arc::new(
        tinyagents_harness::middleware::CliRpcOnlyMiddleware::new(tool_sets.clone()),
    ));

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
        let middleware = if native_security_gate_installed {
            middleware::tool_policy::ToolPolicyMiddleware::new_without_session_ceiling(
                enforcement.policy,
                enforcement.session,
                tool_sets.clone(),
                enforcement.channel,
            )
        } else {
            middleware::tool_policy::ToolPolicyMiddleware::new(
                enforcement.policy,
                enforcement.session,
                tool_sets.clone(),
                enforcement.channel,
            )
        };
        harness.push_tool_middleware(Arc::new(middleware));
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
    harness.push_tool_middleware(Arc::new(
        middleware::credential_scrub::credential_scrub_middleware(),
    ));

    // Malformed-argument recovery (`before_tool`): repair a call's non-object
    // arguments before the crate's schema gate — decode JSON-encoded-string args
    // (optionally markdown-fenced) to an object, or coerce to `{}` only when the
    // tool schema has no required fields (engine parity). A non-object against a
    // required-field schema is left untouched so the crate's
    // `InvalidArgsPolicy::ReturnToolError` admission path reports the original
    // validation error. It never reaches approval/policy wrappers or the tool.
    harness.push_middleware(Arc::new(ArgRecoveryMiddleware::new(tool_sets.clone())));

    // The model's shell `timeout_secs` is when the call comes back, not a kill:
    // move it aside so the clamp below writes the command's deadline there.
    let can_follow = harness
        .tools()
        .names()
        .iter()
        .any(|name| name == "shell_process");
    harness.push_middleware(Arc::new(middleware::shell_yield::ShellYieldMiddleware {
        can_follow,
    }));
    // Clamp shell `timeout_secs` to the turn remainder (#6953), on recovered args.
    harness.push_middleware(Arc::new(shell_turn_budget.clamp()));

    // Bare packed-tool routing (`before_tool`, #6276): a call that names a
    // withheld packed tool directly becomes the `use_skill` call that reaches
    // it, ahead of admission, so every gate above still applies. Only when the
    // session lets that tool run, and never without a session. After
    // `ArgRecoveryMiddleware` so it wraps recovered arguments; before the
    // embedder hooks so they observe the call that actually runs.
    let registered_tools = harness.tools().names();
    harness.push_middleware(Arc::new(
        middleware::packed_tool_route::packed_tool_route_middleware(
            registered_tools,
            route_session,
        ),
    ));

    // Embedder tool lifecycle hooks. Registered AFTER `ArgRecoveryMiddleware`:
    // `before_tool` runs in registration order, so a hook installed earlier would
    // observe the provider's raw (possibly JSON-encoded-string / non-object)
    // arguments instead of the recovered object that `ToolHookContext` documents.
    // The middleware caches the normalized pre-call arguments by `call_id` and
    // replays them into the `PostToolUse` context. Its `after_tool` is
    // observation-only (never mutates the result), so running first in the
    // reverse-order `after_tool` chain is safe — it cannot perturb the
    // summarization/cap or tool-outcome capture layers.
    let mut embedder_tool_hooks = crate::agent::hooks::embedder_tool_hooks_in(&dispatch);
    embedder_tool_hooks.extend(scoped_tool_hooks);
    if !embedder_tool_hooks.is_empty() {
        harness.push_middleware(Arc::new(tinyagents_hooks::tool::ToolHooksMiddleware::<
            super::host::OpenHumanRunContext,
        >::new(embedder_tool_hooks)));
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
    // compacted, trimmed or persisted (`memory::lifecycle::prefetch::pre_turn`).
    harness.push_middleware(Arc::new(middleware::memory_pack::memory_pack_middleware()));

    TurnWiring {
        halt_summary,
        wrap_up_fired,
        compression_mw,
        prompt_cache_guard,
    }
}

fn record_stop_summary(summary: &HaltSummarySlot, iteration: u32, reason: &str) {
    *summary
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!(
        "Stopping after {iteration} model call(s): {reason}"
    ));
}

/// The per-token pricing a resolved route's generation is enriched with: a
/// free model is priced at zero, a catalogued one at its list price, and an
/// unknown one is left unpriced.
fn generation_pricing(model: &str) -> Option<tinyagents_harness::events::GenerationPricing> {
    if crate::agent::cost::is_free_model(model) {
        return Some(tinyagents_harness::events::GenerationPricing {
            input_per_mtok_usd: Some(0.0),
            cached_input_per_mtok_usd: Some(0.0),
            output_per_mtok_usd: Some(0.0),
        });
    }
    tinyagents_registry::pricing::lookup(model).map(|price| {
        tinyagents_harness::events::GenerationPricing {
            input_per_mtok_usd: Some(price.input_per_mtok_usd),
            cached_input_per_mtok_usd: Some(price.cached_input_per_mtok_usd),
            output_per_mtok_usd: Some(price.output_per_mtok_usd),
        }
    })
}

#[cfg(test)]
#[path = "turn_middleware_tests.rs"]
mod tests;
