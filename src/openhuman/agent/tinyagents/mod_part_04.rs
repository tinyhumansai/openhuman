
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
    // Whether this run pauses gracefully at its model-call cap (issue #6014).
    // Only such a run gets the in-loop conclusion: a run that errors at its cap
    // instead (the channel/CLI path, which maps the stop to
    // `MaxIterationsExceeded`) must keep doing that, and handing it a wrap-up
    // would silently convert a documented error into an answer.
    pause_at_cap: bool,
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
    // response, instead of every `native model adapter` pushing it. Installed
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
    // Captured for the same reason `autocompact_enabled` is — `install` consumes
    // `context_mw` — and used to site microcompact below, after compression.
    let microcompact_keep_recent = context_mw.microcompact_keep_recent;
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
        // spawn/delegate tool whatever the allowlist says — re-assert it here at
        // registration, not just on the caller's `allowed_indices`. Warn only when
        // the allowlist actually readmitted one (issue #6157); the caller strips
        // them first, so a bare `spawn_stripped` warn fired on every healthy run.
        let spawn_stripped = is_subagent_run && is_subagent_spawn_or_delegate_tool(name);
        if spawn_stripped && admitted {
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
    match crate::openhuman::agent::registry::agents::load_builtins() {
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
                Box::new(summarize::ModelSummarizer::new(summarizer_model, model)),
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
    }

    // ── The context ladder, cheapest sufficient step first (issue #6014) ──────
    //
    // `before_model` runs in registration order, so what follows IS the firing
    // order, and each step only matters when the one above was not enough:
    // 1. compression (above) folds the older slice into a task-aware summary;
    // 2. microcompact blanks older tool bodies if still over;
    // 3. the wrap-up undoes (2) for a capped turn's concluding call;
    // 4. trim evicts oldest whole messages if still over.
    //
    // The order used to be 2 -> 1 -> 4, because `context_mw.install` registered
    // microcompact: the summarizer was handed a transcript whose tool bodies
    // were already `CLEARED_PLACEHOLDER` and asked, by its own prompt, for "key
    // results/outputs" it could no longer see. Nothing recovered them.
    if microcompact_keep_recent > 0 {
        // The token-budget gate the crate added for this (#4755) and this call
        // site never used. Constructed bare, microcompact blanks on EVERY call
        // past `keep_recent` — not only under pressure — so a ten-call turn
        // answered from the last five results while well inside a window with
        // room for all ten. Not a capped-turn problem: every turn. The budget
        // matches the trim's allowance, putting microcompact strictly behind
        // compression. With no window there is nothing to size against and
        // nothing else bounding growth, so always-blank stays as the backstop.
        let microcompact = tinyagents_harness::middleware::MicrocompactMiddleware::new(
            microcompact_keep_recent,
            crate::openhuman::agent::context::CLEARED_PLACEHOLDER,
        );
        let microcompact = match context_window.filter(|w| *w > 0) {
            Some(window) => {
                microcompact.with_token_budget(middleware::legacy_max_input_tokens(window).max(1))
            }
            None => microcompact,
        };
        // Emit `AgentEvent::Compressed` when a body is cleared. Off by default —
        // the middleware was built as "a silent transcript rewrite" — which is
        // why blanking was, until now, the one reduction step nobody could see
        // happening: compression logs its provenance, the trim warns on every
        // eviction, and the per-result artifact store logs each persist. Only
        // this one destroyed content without saying so, which is how it went
        // unnoticed that it was doing it on every call.
        harness.push_middleware(Arc::new(microcompact.with_events(true)));
    }

    // Issue #6014: make the last permitted model call the turn's conclusion,
    // rather than leaving the answer to an extra out-of-band call after the loop
    // has exited.
    //
    // **Top-level turns only**, the same cut `CapPauser`'s dispatch guard takes.
    // A sub-agent reaching its own cap is a routine outcome, not a user-visible
    // dead end: it summarises, hands the result back to its parent, and
    // `subagent_runner` already owns that checkpoint. The harm this fixes — a
    // person left with a status line where an answer should be — belongs to the
    // turn that answers a human. It also leaves a child's budget alone: the
    // conclusion costs a call, and turning every delegated run's N tool rounds
    // into N-1 is not a trade to impose as a side effect.
    // One allowance, split between the two things that add to the request
    // (CodeRabbit on #6068). Computed independently they were each bounded and
    // the pair was not: restoration could fill the whole allowance and the
    // contents list add its tenth on top. `0` when no window is advertised.
    let trim_allowance = context_window
        .filter(|w| *w > 0)
        .map(|w| middleware::legacy_max_input_tokens(w).max(1))
        .unwrap_or(0);
    let (toc_allowance, restore_allowance) = middleware::split_input_allowance(trim_allowance);

    let wrap_up_mw = (pause_at_cap && subagent_scope.is_none()).then(|| {
        Arc::new(middleware::FinalCallWrapUpMiddleware::new(
            crate::openhuman::agent::harness::session::turn_checkpoint::MAX_ITER_CHECKPOINT_INSTRUCTION,
            tool_outcome_sink.clone(),
            // What is left after the contents list's share, so restoration
            // stops short of provoking an eviction (see the middleware).
            restore_allowance,
        ))
    });
    let wrap_up_fired = wrap_up_mw.as_ref().map(|mw| mw.fired());
    if let Some(mw) = wrap_up_mw {
        harness.push_middleware(mw);
    }

    // Issue #6014: say where the offloaded results went, from the index rather
    // than the transcript. After the reduction steps so it renders what
    // survived them, before the trim so its message is counted in that budget.
    // Its share of the allowance split above: the list is a system message, so
    // nothing downstream can shrink it (see the middleware).
    harness.push_middleware(Arc::new(middleware::ArtifactIndexTocMiddleware::new(
        toc_allowance,
    )));

    if let Some(window) = context_window.filter(|w| *w > 0) {
        // Deterministic hard-cap trim (issue #4462), last in the ladder: it drops
        // whole messages, so it runs only once summarizing and blanking have both
        // failed to fit the window. See `ImageAwareMessageTrimMiddleware` for the
        // three legacy guards it restores over the crate trim.
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
    // required-field schema is left untouched so the crate's
    // `InvalidArgsPolicy::ReturnToolError` admission path reports the original
    // validation error. It never reaches approval/policy wrappers or the tool.
    harness.push_middleware(Arc::new(middleware::ArgRecoveryMiddleware::new(
        tool_sets.clone(),
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
    let embedder_tool_hooks = crate::openhuman::agent::hooks::embedder_tool_hooks();
    if !embedder_tool_hooks.is_empty() {
        harness.push_middleware(Arc::new(middleware::EmbedderToolHooksMiddleware::new(
            embedder_tool_hooks,
        )));
    }

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
