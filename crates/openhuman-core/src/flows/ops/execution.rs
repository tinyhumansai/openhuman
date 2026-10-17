use super::*;

/// Builds the `TrustedAutomation { Workflow }` origin scoped around every
/// `flows_run` / `flows_resume` invocation. See `flows_run`'s doc for why
/// this applies uniformly regardless of caller.
pub(super) fn workflow_origin(flow_id: &str, require_approval: bool) -> AgentTurnOrigin {
    AgentTurnOrigin::TrustedAutomation {
        job_id: flow_id.to_string(),
        source: TrustedAutomationSource::Workflow { require_approval },
    }
}

/// Reads a settled run's durable [`tinyflows::engine::GraphObservation`]
/// slice back out of the per-run journal (keyed by the tinyagents-minted
/// `graph_run_id`) and exports it to Langfuse as one trace. Best-effort by
/// construction: any journal read failure is logged and swallowed, and the
/// exporter itself never fails the run. Skips the journal read entirely when
/// `observability.share_usage_data` is off.
pub(super) async fn export_run_to_langfuse(
    config: &Config,
    flow_name: &str,
    flow_id: &str,
    thread_id: &str,
    status: &str,
    trigger: FlowRunTrigger,
    journal: &tinyflows::engine::InMemoryGraphEventJournal,
    graph_run_id: &str,
) {
    if !config.observability.share_usage_data {
        tracing::debug!(
            target: "flows",
            flow_id = %flow_id,
            "[flows] langfuse export skipped: observability.share_usage_data is off"
        );
        return;
    }
    use tinyflows::engine::GraphEventJournal as _;
    let observations = match journal.read_from(graph_run_id, 0).await {
        Ok(observations) => observations,
        Err(e) => {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                %thread_id,
                graph_run_id = %graph_run_id,
                error = %e,
                "[flows] langfuse export skipped: could not read run journal"
            );
            return;
        }
    };
    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        %thread_id,
        graph_run_id = %graph_run_id,
        observation_count = observations.len(),
        "[flows] exporting flow run trace to Langfuse"
    );
    crate::flows::tinyflows::langfuse_export::export_flow_run_trace(
        config,
        flow_name,
        flow_id,
        thread_id,
        status,
        trigger,
        &observations,
    )
    .await;
}

/// Executes an already-prepared, already-`running`-row-inserted flow run to a
/// terminal state, finalizing the `flow_runs` row on every exit path.
///
/// Split out of [`flows_run`] (bugs B41/B42) so the synchronous and detached
/// entry points share ONE run body — and so a single [`RunRowFinalizer`]
/// reconciles the row to `"interrupted"` if this future is dropped mid-await
/// before any terminal write lands. The caller MUST have already
/// [`run_registry::register`]ed `thread_id` (handing the token + guard in
/// here), inserted the initial `running` row ([`start_flow_run_row`]) and
/// published `FlowRunStarted`.
///
/// **Registration is the caller's job on purpose.** It used to happen here, but
/// on the detached path that left a window: `flows_run_detached` returned the
/// `run_id` to the agent before the spawned task had registered, so a
/// `flows_cancel_run` landing in that gap saw `is_in_flight == false`, took the
/// "parked/stale" branch, wrote a terminal `cancelled` row and dropped the
/// checkpoint — while this body then started and executed the flow's real
/// side effects anyway, finally overwriting `cancelled` with its own terminal
/// status. Registering before the `run_id` is observable makes the cancel
/// always take the signalled branch instead. `_run_guard` is held for the whole
/// body and deregisters on any exit, including the early returns below.
///
/// Returned boxed and `#[inline(never)]` on purpose: an `async fn` body is
/// otherwise re-instantiated inside every crate / codegen unit that awaits it,
/// and this state machine is large. Boxing here keeps one copy, compiled in
/// this crate.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(super) fn run_flow_body(
    ctx: crate::core::runtime::DispatchCtx,
    config_arc: Arc<Config>,
    flow: Flow,
    flow_id: String,
    thread_id: String,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
    no_actionable_nodes: bool,
    cancel_token: tokio_util::sync::CancellationToken,
    _run_guard: run_registry::RunGuard,
) -> futures::future::BoxFuture<'static, Result<Outcome<Value>, String>> {
    Box::pin(run_flow_body_inner(
        ctx,
        config_arc,
        flow,
        flow_id,
        thread_id,
        input,
        inputs,
        trigger,
        no_actionable_nodes,
        cancel_token,
        _run_guard,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn run_flow_body_inner(
    ctx: crate::core::runtime::DispatchCtx,
    config_arc: Arc<Config>,
    flow: Flow,
    flow_id: String,
    thread_id: String,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
    no_actionable_nodes: bool,
    cancel_token: tokio_util::sync::CancellationToken,
    _run_guard: run_registry::RunGuard,
) -> Result<Outcome<Value>, String> {
    let config: &Config = config_arc.as_ref();
    let flow_id: &str = flow_id.as_str();

    // B42 drop-guard, armed BEFORE the first `.await` in this body (R-M5).
    //
    // The caller has already inserted the `running` row, so every await from
    // here on is a window in which dropping this future would strand that row.
    // The guard used to be constructed ~150 lines below, immediately around the
    // engine call — which left the inference-readiness preflight directly below
    // (a real network probe on a cache miss) unguarded: a client disconnect or
    // an aborted detached task during that probe dropped the future before any
    // finalizer existed, and the row stayed a perpetual `running` spinner until
    // the NEXT process boot sweep (the in-process one had already run). Arming
    // it here covers the whole awaiting region; every settled path below still
    // disarms it after its own terminal write.
    let finalizer = RunRowFinalizer::new(config_arc.clone(), &thread_id, flow_id);

    // B45 run-time preflight (design correction — see the "Inference-readiness
    // check" module doc above): an `agent` node needs a working LLM provider
    // to run at all, but that is no longer enforced as an author-time gate —
    // `propose_workflow`/`edit_workflow`/`save_workflow` always succeed now,
    // so a graph can reach here whose agent node(s) cannot currently complete.
    // Catch that HERE, before the tinyflows engine (and any upstream
    // fetch/prep nodes) does real work for nothing, and finalize the run row
    // as `failed` with a clear, actionable message instead of the opaque,
    // several-layers-deep "capability error: graph error: capability error:
    // model error: ... API key not configured for provider" a mid-run failure
    // surfaces as. Reuses `validate_inference_readiness` — backed by the same
    // cached evaluation `build_builder_proposal`'s advisory `inference_status`
    // warns on — so a run right after a proposal/edit reads the cached
    // negative (`INFERENCE_PROBE_CACHE`) instead of re-probing the network.
    // Returns an empty `Vec` (no-op here) for a tool_call-only graph, and is
    // never consulted by `dry_run_workflow` (sandbox runs are exempt by
    // design — that tool doesn't route through `run_flow_body` at all).
    let inference_errors = validate_inference_readiness(&ctx, config, &flow.graph).await;
    if !inference_errors.is_empty() {
        let detail = inference_errors.join(" ");
        let msg = format!("This flow's AI step needs a working AI provider to run. {detail}");
        tracing::warn!(
            target: "flows",
            flow_id,
            "[flows] run_flow_body: inference-readiness preflight failed — finalizing run as \
             failed without invoking the engine: {msg}"
        );
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id,
                error = %rec_err,
                "[flows] run_flow_body: failed to record failed run (inference preflight)"
            );
        }
        let observed = current_persisted_steps(config, &thread_id);
        finish_flow_run_row(
            config,
            &thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(&msg),
            None,
        );
        finalizer.disarm();
        return Err(msg);
    }

    // Recompile to execute — the entry point already compile-checked to fail
    // fast before the running row existed. A failure *now* (after the row was
    // inserted) must finalize the row as failed, never orphan it.
    let compiled = match tinyflows::compiler::compile(&flow.graph) {
        Ok(compiled) => compiled,
        Err(e) => {
            let msg = e.to_string();
            tracing::warn!(target: "flows", flow_id, error = %msg, "[flows] run_flow_body: compile failed after start row inserted");
            let observed = current_persisted_steps(config, &thread_id);
            finish_flow_run_row(
                config,
                &thread_id,
                flow_id,
                "failed",
                &observed,
                &[],
                Some(&msg),
                None,
            );
            finalizer.disarm();
            return Err(msg);
        }
    };

    // Scope the state store per-flow so two flows never collide on a state key.
    let flow_run = FlowRunContext {
        flow_id: flow_id.to_string(),
        run_id: thread_id.clone(),
    };
    // The run acts under its own trusted `Workflow` origin: its capabilities
    // read the origin from this context, not from the calling task's scope.
    let origin = workflow_origin(flow_id, flow.require_approval);
    let run_ctx = ctx.with_turn_origin(origin.clone());
    let caps = crate::flows::tinyflows::build_capabilities(
        &run_ctx,
        config_arc.clone(),
        format!("flow:{flow_id}"),
        Some(flow_run),
    );
    let checkpointer = match crate::flows::tinyflows::open_flow_checkpointer(config) {
        Ok(checkpointer) => checkpointer,
        Err(e) => {
            let msg = e.to_string();
            tracing::warn!(target: "flows", flow_id, error = %msg, "[flows] run_flow_body: checkpointer open failed after start row inserted");
            let observed = current_persisted_steps(config, &thread_id);
            finish_flow_run_row(
                config,
                &thread_id,
                flow_id,
                "failed",
                &observed,
                &[],
                Some(&msg),
                None,
            );
            finalizer.disarm();
            return Err(msg);
        }
    };

    // Record a failed attempt so `last_run_at`/`last_status` reflect reality
    // (a stop-policy engine/capability failure or a timeout) rather than
    // leaving the prior success/pending state on the flow. Preserve whatever
    // steps the observer persisted live (don't wipe them back to `[]`).
    let record_failed = |error: &str| {
        if let Err(rec_err) = store::record_run(config, flow_id, "failed") {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                error = %rec_err,
                "[flows] flows_run: failed to record failed run"
            );
        }
        let observed = current_persisted_steps(config, &thread_id);
        finish_flow_run_row(
            config,
            &thread_id,
            flow_id,
            "failed",
            &observed,
            &[],
            Some(error),
            None,
        );
    };

    // Per-run in-memory journal: tinyflows records every graph event as a
    // durable GraphObservation under the run's tinyagents run id, which the
    // post-run Langfuse export reads back. Process-local and dropped with the
    // run — never persisted.
    let journal = Arc::new(tinyflows::engine::InMemoryGraphEventJournal::new());
    // Live run observer (issue G2): persists each finished step into the
    // `flow_runs` row as it happens and streams a `FlowRunProgress` event to
    // the frontend, so the durable + journaled path also reports live.
    let observer: Arc<dyn tinyflows::observability::RunObserver> = Arc::new(
        crate::flows::tinyflows::observability::FlowRunObserver::new(
            Arc::new(config.clone()),
            flow_id,
            thread_id.clone(),
        ),
    );
    // The flow/run correlation (issue flow-approval-surface, PR2) rides the
    // capabilities built above, next to the `Workflow` origin scoped here, so a
    // tool call the engine dispatches can, if it parks in the `ApprovalGate`,
    // stamp its `PendingApproval` with `source_context = Flow { flow_id,
    // run_id }` -- the origin alone only carries `flow_id`.
    let run = with_origin(
        origin,
        tinyflows::engine::run_with_checkpointer_journaled_observed(
            &compiled,
            tinyflows::engine::RunInput::new(input).with_inputs(inputs),
            &caps,
            checkpointer,
            &thread_id,
            journal.clone(),
            &observer,
        ),
    );
    let timed = tokio::time::timeout(std::time::Duration::from_secs(FLOW_RUN_TIMEOUT_SECS), run);
    tokio::pin!(timed);
    // (The B42 drop-guard is armed near the top of this fn, before the first
    // `.await` — see `finalizer` there.)
    // Race the run against a cancellation signal (issue G4). `biased` checks the
    // cancel arm first so a `flows_cancel_run` that lands right as the run
    // settles still wins deterministically.
    let journaled = tokio::select! {
        biased;
        _ = cancel_token.cancelled() => {
            tracing::info!(target: "flows", flow_id = %flow_id, thread_id = %thread_id, "[flows] flows_run: cancelled mid-run");
            if let Err(e) = store::record_run(config, flow_id, "cancelled") {
                tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_run: failed to record cancelled run");
            }
            let observed = current_persisted_steps(config, &thread_id);
            finish_flow_run_row(
                config,
                &thread_id,
                flow_id,
                "cancelled",
                &observed,
                &[],
                Some("run cancelled"),
                None,
            );
            finalizer.disarm();
            drop_checkpoint(config, &thread_id).await;
            return Ok(Outcome::single_log(
                json!({
                    "output": Value::Null,
                    "pending_approvals": Vec::<String>::new(),
                    "thread_id": thread_id,
                    "cancelled": true,
                }),
                format!("flow run cancelled: {thread_id}"),
            ));
        }
        result = &mut timed => match result {
            Ok(Ok(journaled)) => journaled,
            Ok(Err(e)) => {
                record_failed(&e.to_string());
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, error = %e, "[flows] flows_run: run failed");
                return Err(e.to_string());
            }
            Err(_elapsed) => {
                let msg = format!("flow run timed out after {FLOW_RUN_TIMEOUT_SECS}s");
                record_failed(&msg);
                finalizer.disarm();
                tracing::warn!(target: "flows", flow_id = %flow_id, timeout_secs = FLOW_RUN_TIMEOUT_SECS, "[flows] flows_run: run timed out");
                return Err(msg);
            }
        },
    };
    let outcome = journaled.outcome;

    let settled = settle_steps(config, &thread_id, &outcome.output);
    let (status, error) = finalize_terminal_status(&settled, &outcome.pending_approvals);
    // T-M1: pin the graph this run just executed only on the write that parks
    // it — `flows_resume` recomputes and compares this hash against the
    // *current* flow graph before it will honour the approval. See
    // `compute_graph_hash`'s doc.
    let graph_hash = (status == "pending_approval")
        .then(|| compute_graph_hash(&flow.graph, flow.require_approval))
        .flatten();
    // Finalize the run row (and disarm the drop-guard) BEFORE the flow-summary
    // write, so a `record_run` failure can never leave the row wedged at
    // `running` — the row's terminal state is the correctness-critical write;
    // the summary is best-effort observability (see `start_flow_run_row`).
    finish_flow_run_row(
        config,
        &thread_id,
        flow_id,
        status,
        &settled,
        &outcome.pending_approvals,
        error.as_deref(),
        graph_hash.as_deref(),
    );
    finalizer.disarm();
    if let Err(e) = store::record_run(config, flow_id, status) {
        tracing::warn!(target: "flows", flow_id = %flow_id, status, error = %e, "[flows] flows_run: failed to record run summary (run row already finalized)");
    }
    export_run_to_langfuse(
        config,
        &flow.name,
        flow_id,
        &thread_id,
        status,
        trigger,
        &journal,
        &journaled.graph_run_ids.run_id,
    )
    .await;
    notify_pending_approval(&flow, &thread_id, &outcome.pending_approvals);

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        status,
        pending_approvals = outcome.pending_approvals.len(),
        no_actionable_nodes,
        "[flows] flows_run: finished"
    );

    const NO_ACTIONABLE_NODES_NOTE: &str = "This flow's graph has no actionable nodes beyond \
         its trigger (no downstream action nodes, or no edges connecting them) — the run \
         completed without doing anything. Add and wire up at least one action node.";

    let mut result = json!({
        "output": outcome.output,
        "pending_approvals": outcome.pending_approvals,
        "thread_id": thread_id,
    });
    let mut logs = vec![format!("flow run {status}")];
    if no_actionable_nodes {
        result["note"] = json!(NO_ACTIONABLE_NODES_NOTE);
        logs.push(NO_ACTIONABLE_NODES_NOTE.to_string());
    }

    Ok(Outcome::new(result, logs))
}

/// Milliseconds since the Unix epoch, for `CoreNotificationEvent::timestamp_ms`.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Surfaces a paused run as a `CoreNotification` (category `Agents`) with an
/// "approve" action carrying `flow_id`/`thread_id`/`node_ids`, mirroring the
/// pattern `agent_meetings::calendar`'s auto-summarize "Ask" flow uses
/// (direct `publish_core_notification` call with an action payload, not the
/// generic `DomainEvent -> event_to_notification` bridge — this is a
/// flows-specific card with flow-specific action data, not a translation of
/// an existing broadcast event). No-op when nothing is pending.
pub(super) fn notify_pending_approval(flow: &Flow, thread_id: &str, pending_approvals: &[String]) {
    if pending_approvals.is_empty() {
        return;
    }

    use crate::desktop::notifications::bus::publish_core_notification;
    use crate::desktop::notifications::types::{
        CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
    };

    let action_payload = json!({
        "flow_id": flow.id,
        "thread_id": thread_id,
        "node_ids": pending_approvals,
    });

    publish_core_notification(CoreNotificationEvent {
        id: format!("flow-pending-approval:{}:{}", flow.id, thread_id),
        category: CoreNotificationCategory::Agents,
        title: "Workflow needs approval".to_string(),
        body: format!(
            "\"{}\" is waiting on {} approval{} before it can continue.",
            flow.name,
            pending_approvals.len(),
            if pending_approvals.len() == 1 {
                ""
            } else {
                "s"
            }
        ),
        // No dedicated Workflows review route exists yet (B3 ships the UI);
        // leave unset rather than link to a page that can't act on it.
        deep_link: None,
        timestamp_ms: now_ms(),
        actions: Some(vec![CoreNotificationAction {
            action_id: "approve".to_string(),
            label: "Review".to_string(),
            payload: Some(action_payload),
        }]),
        workspace: None,
        workspace_revision: None,
    });
}
