
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
    // stamps it onto every `ModelRequest.max_tokens` and the native model adapter
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

    let input = crate::openhuman::agent::message_convert::history_to_messages(&history);
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
    let cancellation = tinyagents_harness::CancellationToken::new();
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
    if let Some(session_kv) =
        crate::openhuman::agent::session_import::live::session_kv_store().await
    {
        stores.get_or_insert_with(StoreRegistry::new).register(
            crate::openhuman::agent::session_import::live::TINYAGENTS_SESSION_KV_STORE,
            session_kv,
        );
        tracing::debug!(
            "[session-store] registered session kv store on RunContext.stores under '{}'",
            crate::openhuman::agent::session_import::live::TINYAGENTS_SESSION_KV_STORE
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
    //
    // It is also handed the turn's dispatch guard, so the pause is *recorded* and
    // not merely requested. `SteeringCommand::Pause` is advisory — honoured at the
    // loop boundary — and nothing consulted it before dispatching a new sub-agent,
    // so a dispatch issued in the same instant raced it and took the whole turn
    // down with the run's remaining wall-clock budget (#5804). The guard is
    // resolved here rather than inside the listener because this future runs on
    // the turn's task, where the task-local is in scope; the listener need not.
    if pause_at_cap {
        if let (Some(events), Some(handle)) = (&events, &handle) {
            // Only the TOP-LEVEL turn's cap pause is binding on dispatch. A
            // sub-agent reaching its own model-call cap is a routine outcome —
            // it summarises and hands its result back (`hit_cap`) — and the
            // parent may legitimately keep delegating afterwards. Recording a
            // child's cap here would stop the whole turn's fan-out on a signal
            // that says nothing about the parent's budget, so `subagent_scope`
            // gates it: `None` is the chat turn, `Some` is a delegated child.
            // The child still gets its advisory `Pause` either way.
            let dispatch_guard = subagent_scope
                .is_none()
                .then(crate::openhuman::agent::harness::turn_dispatch_guard::current)
                .flatten();
            events.subscribe(CapPauser::new(
                handle.clone(),
                max_iterations,
                dispatch_guard,
            ));
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
        .map(|scope| tinyagents_harness::ids::ThreadId::new(scope.task_id.clone()));
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
                        terminal = Some(Err(tinyagents_harness::TinyAgentsError::Model(error)));
                        break;
                    }
                }
            }
            terminal.unwrap_or_else(|| {
                Err(tinyagents_harness::TinyAgentsError::Model(
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
            // `native model adapter::chat`/`stream`), so a stale slot should not exist
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
            if let tinyagents_harness::TinyAgentsError::LimitExceeded(msg) = &e {
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
    // reason) built by `ModelSummarizer`. Surfaced at info with a
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
        // NOT best-effort. `TurnCompleted` is the web bridge's sole completion
        // signal: drop it and `parent_completed` stays false, so the bridge
        // marks a turn that actually finished as `interrupted` and never emits
        // `chat_done`. The turn's output still reaches the journal, session
        // transcript and memory tree, so the agent "remembers" replying while
        // the user's thread shows silence. A heavy turn (many tools + long
        // streaming) reliably fills the 256-slot channel, which is why only
        // tool-heavy turns were affected.
        //
        // Blocking is safe *here specifically*: this site is guarded by
        // `subagent_scope.is_none()`, so it only ever runs on a parent turn
        // with nothing awaiting it. The sub-agent stall documented on
        // `tool_progress::emit` comes from parking a *sub-agent's* loop while
        // the orchestrator awaits its tool call — unreachable from this path.
        // Deltas and sub-agent lifecycle events stay lossy via `emit`.
        if let Err(err) = sink
            .send(AgentProgress::TurnCompleted {
                iterations: run.model_calls as u32,
            })
            .await
        {
            tracing::warn!(
                error = %err,
                "[tinyagents] TurnCompleted not delivered — progress receiver gone"
            );
        }
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
            let charged = crate::openhuman::platform::cost::catalog::estimate_cost_usd(
                model, input, output, cached,
            );
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

    let conversation = crate::openhuman::agent::message_convert::messages_to_conversation(
        crate::openhuman::agent::message_convert::messages_since_request(
            &run.messages,
            request_base_len,
        ),
    );
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
        history: crate::openhuman::agent::message_convert::messages_to_history(&run.messages),
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
