use super::*;

#[path = "drive/controls.rs"]
mod controls;
#[path = "drive/setup.rs"]
mod setup;
#[path = "drive/stores.rs"]
mod stores;

pub(super) async fn drive_turn(
    spec: TurnSpec<'_>,
    hosted_root: Option<(
        Arc<crate::agent::tinyagents::host::OpenHumanHostBase>,
        String,
    )>,
    native_subagent: Option<NativeSubagentRunContext>,
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
        message_run_queue,
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
    let native_security_gate = if hosted_root.is_none() {
        native_subagent.as_ref().and_then(|native| {
            run_context.hosted_base.as_ref().map(|base| {
                let mut gate = crate::agent::tinyagents::host::OpenHumanSecurityGate::new(
                    base.security_policy.clone(),
                    tool_sets.clone(),
                )
                .with_input_screening(!run_context.untrusted_input_turn)
                .with_approval(run_context.approval.clone())
                .with_dispatch(run_context.dispatch_ctx())
                .with_headless_descendant_settlement(run_context.headless_descendant_settlement);
                if let Some(policy) = tool_policy.as_ref() {
                    gate = gate.with_tool_policy(Arc::new(policy.session.clone()));
                }
                (Arc::new(gate), native.name.clone())
            })
        })
    } else {
        None
    };
    let is_hosted_root = hosted_root.is_some();
    let hosted_security_gate = is_hosted_root || native_security_gate.is_some();
    let is_local = turn_models.is_local();
    // Wrap concrete routes before the invocation-local hosted resolver captures them.
    let turn_models = turn_models.with_host_controls(
        run_context.model_budget.as_ref(),
        run_context.dispatch_admission.clone(),
    );
    let hosted_model_resolver = hosted_root.as_ref().map(|_| {
        Arc::new(turn_models.resolver()) as Arc<dyn tinyagents_harness::host::ModelResolver<()>>
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
        crate::agent::tinyagents::thread_reasoning::turn_reasoning_for(
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
        summarizer_model_id,
        summarizer_settings,
        error_slot,
        ..
    } = turn_models;
    let plan_models =
        turn_models_for_plan(model, primary, tier_routes, &error_slot, is_hosted_root);
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
    let (config, journal_run_id) = setup::run_config(
        &mut run_context,
        native_subagent.as_ref(),
        is_subagent,
        on_progress.is_some(),
        max_iterations,
        max_output_tokens,
        model,
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
    if native_subagent.is_some() {
        // The prepared child context carries the parent's sink. Never attach
        // child model/tool records to it: those records have run-local offsets
        // and the parent run id. Emit the lifecycle link through that sink.
        ctx.events = EventSink::with_stream_id(journal_run_id.as_str());
    }
    let run_instance_id = ctx.instance_id();
    ctx = if let Some(stores) =
        stores::attach(&run_context, tool_result_artifact_index.clone()).await
    {
        ctx.with_stores(stores)
    } else {
        ctx
    };

    let streaming = on_progress.is_some();
    let turn_completed_sink =
        setup::turn_completed_sink(&on_progress, is_subagent, defer_turn_completed_to_caller);
    let sink = setup::event_sink(&journal_run_id, native_subagent.as_ref());
    let bridge = setup::event_bridge(
        &sink,
        on_progress,
        model,
        provider_id.clone(),
        max_iterations,
        subagent_scope.clone(),
        &cursor,
        &tool_names,
        &failure_map,
        &provider_usage_carry,
        bridge_tool_sets.clone(),
        &run_context,
    );

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
        ctx.config.lineage.parent_run_id.clone(),
        ctx.config.lineage.root_run_id.clone(),
    )
    .await;
    if let Some(native) = &native_subagent {
        *native
            .child_journal
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = turn_journal.clone();
    }
    if !is_subagent {
        if let Some(crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat {
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

    let controls = controls::build(
        max_iterations,
        pause_at_cap,
        early_exit_tools,
        subagent_scope.clone(),
        message_run_queue,
        run_queue,
        is_subagent,
        &run_context,
    );
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
            provider_id: provider_id.clone(),
            model: model.to_string(),
            tool_sets,
            allowed,
            candidate_names,
            subagent_scope: subagent_scope.clone(),
            context_window,
            context_mw,
            host_policy: run_context.harness_host_policy(),
            tool_policy: tool_policy.clone(),
            required_capabilities: tinyagents_registry::WorkloadRouter::from_workload_routes(
                "hint:vision",
            )
            .required_capabilities(model),
            deterministic_cacheable,
            has_route_fallback,
            dispatch: run_context.dispatch_ctx(),
            hosted_security_gate,
            native_security_gate,
            pause_at_cap,
            run_mode: run_context.thread_id.as_deref().map(|thread_id| {
                crate::threads::run_mode::handle_for_thread(&run_context.dispatch_ctx(), thread_id)
            }),
            memory_turn: run_context.memory_turn.clone(),
            summarizer: crate::agent::tinyagents::context_wiring::CompactionSummarizer {
                model: summarizer_model,
                model_id: summarizer_model_id,
                settings: summarizer_settings,
            },
            failure_map: failure_map.clone(),
            provider_usage_carry: provider_usage_carry.clone(),
            tool_outcome_sink: tool_outcome_sink.clone(),
            // The store the `todo` tool resolves from the same parent context.
            todo_store: run_context
                .parent
                .as_ref()
                .filter(|_| todo_registered)
                .map(|parent| {
                    crate::storage::sessions::stores_or_ephemeral(&parent.workspace_dir).kv
                }),
            compaction_notice: Some({
                let bridge = bridge.clone();
                Arc::new(move || bridge.announce_compaction())
            }),
            state_store: run_context.parent.as_ref().map(|parent| {
                (
                    crate::storage::sessions::stores_or_ephemeral(&parent.workspace_dir).kv,
                    parent.session_id.clone(),
                )
            }),
            goal_suppressed: run_context.goal_suppressed,
        },
    );
    if let Some(scope) = run_context.response_shape.clone() {
        tinyagents_harness::middleware::install_response_shape(&mut prepared.harness, scope);
    }
    super::super::budget::install_depth(&mut prepared.harness, &run_context);
    super::super::deadline_wind_down::install(
        &mut prepared.harness,
        &Some(steering.clone()),
        &run_context,
        &subagent_scope,
    );
    turn_observer::install(&mut prepared.harness, &sink, is_hosted_root);

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
                    model_resolver: hosted_model_resolver
                        .expect("hosted root retained its selected model resolver"),
                },
                &run_context,
            );
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
        .with_replayed_prefix(request_base_len.saturating_sub(1))
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
            super::super::hosted_error::run_error_from_hosted(error)
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
    let outcome = finalize_turn_outcome(
        run,
        model,
        max_iterations,
        &subagent_scope,
        pause_at_cap,
        turn_journal.as_ref(),
        &sink,
        defer_turn_completed_to_caller || native_subagent.is_some(),
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
        journal_run_id,
    )
    .await;
    Ok(outcome)
}
