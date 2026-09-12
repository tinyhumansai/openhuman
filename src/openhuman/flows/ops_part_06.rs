
/// Registers (or refreshes) the `cron` job backing a `schedule`-trigger
/// flow. Idempotent — re-uses an existing binding via
/// `cron::find_flow_schedule_job` rather than creating a duplicate, so this
/// is safe to call both from `flows_set_enabled` and from boot
/// reconciliation ([`reconcile_schedule_triggers_on_boot`]).
fn bind_schedule_trigger(config: &Config, flow: &Flow) {
    let Some(trigger_config) = bus::extract_trigger_config(flow) else {
        tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger: flow has no single trigger node — cannot bind cron job");
        return;
    };
    let Some(schedule_raw) = trigger_config.get("schedule").cloned() else {
        tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger config is missing `schedule` — cannot bind cron job");
        return;
    };
    let schedule: crate::openhuman::cron::Schedule = match serde_json::from_value(schedule_raw) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] invalid schedule trigger config — cannot bind cron job");
            return;
        }
    };

    match crate::openhuman::cron::find_flow_schedule_job(config, &flow.id) {
        Ok(Some(existing)) => {
            let patch = crate::openhuman::cron::CronJobPatch {
                enabled: Some(true),
                schedule: Some(schedule),
                ..Default::default()
            };
            if let Err(e) = crate::openhuman::cron::update_job(config, &existing.id, patch) {
                tracing::warn!(target: "flows", flow_id = %flow.id, cron_job_id = %existing.id, error = %e, "[flows] failed to refresh existing schedule-trigger cron job");
            } else {
                tracing::debug!(target: "flows", flow_id = %flow.id, cron_job_id = %existing.id, "[flows] refreshed existing schedule-trigger cron job");
            }
        }
        Ok(None) => match crate::openhuman::cron::add_flow_schedule_job(config, &flow.id, schedule)
        {
            Ok(job) => {
                tracing::info!(target: "flows", flow_id = %flow.id, cron_job_id = %job.id, "[flows] registered schedule-trigger cron job")
            }
            Err(e) => {
                tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] failed to register schedule-trigger cron job")
            }
        },
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] failed to look up existing schedule-trigger cron job");
        }
    }
}

/// Removes the `cron` job backing a `schedule`-trigger flow, if one exists.
fn unbind_schedule_trigger(config: &Config, flow_id: &str) {
    match crate::openhuman::cron::find_flow_schedule_job(config, flow_id) {
        Ok(Some(job)) => {
            if let Err(e) = crate::openhuman::cron::remove_job(config, &job.id) {
                tracing::warn!(target: "flows", %flow_id, cron_job_id = %job.id, error = %e, "[flows] failed to remove schedule-trigger cron job");
            } else {
                tracing::debug!(target: "flows", %flow_id, cron_job_id = %job.id, "[flows] removed schedule-trigger cron job");
            }
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(target: "flows", %flow_id, error = %e, "[flows] failed to look up schedule-trigger cron job for teardown");
        }
    }
}

/// Webhook trigger binding is a documented B2 stub (best-effort deviation):
/// registering a real inbound route requires provisioning a backend tunnel
/// (`webhooks::ops::create_tunnel`, a network call to the signed-in backend
/// account) plus a UI surface to show the resulting URL to the user — both
/// are B3 territory. Rather than silently doing nothing, this logs a clear,
/// actionable warning every time a `webhook`-trigger flow is enabled/disabled
/// so the gap is diagnosable. `flows::bus::FlowTriggerSubscriber` logs the
/// matching deferral on the inbound side (`WebhookIncomingRequest`).
fn log_webhook_trigger_deferred(flow: &Flow, enabled: bool) {
    tracing::warn!(
        target: "flows",
        flow_id = %flow.id,
        enabled,
        "[flows] webhook trigger binding is not implemented in B2 (requires backend tunnel \
         provisioning + a UI surface for the resulting URL) — this flow will not fire \
         automatically from an inbound webhook until that lands"
    );
}

/// Boot-time reconciliation: registers the `cron` job for every enabled,
/// `schedule`-trigger flow. Idempotent (delegates to [`bind_schedule_trigger`],
/// which re-uses an existing binding) — mirrors
/// `cron::seed::seed_proactive_agents_on_boot`'s "ensure jobs exist for
/// already-onboarded users upgrading from an older build" pattern, so a
/// flow enabled on a build that predates this cron binding (or whose binding
/// was lost some other way) gets its schedule re-registered on the next
/// boot without the user having to toggle it off and on.
pub async fn reconcile_schedule_triggers_on_boot(config: &Config) -> Result<(), String> {
    let (flows, skipped) = store::list_enabled_flows(config).map_err(|e| e.to_string())?;
    if skipped > 0 {
        // R-M4: a corrupt/unmigratable row must not abort boot reconciliation
        // for every other enabled flow — skipped rows are logged loudly
        // (never their content) so the gap is diagnosable.
        tracing::warn!(target: "flows", skipped, "[flows] reconcile_schedule_triggers_on_boot: skipped corrupt/unmigratable flow rows");
    }
    let mut reconciled = 0usize;
    for flow in &flows {
        if matches!(bus::extract_trigger_kind(flow), Some(TriggerKind::Schedule)) {
            bind_schedule_trigger(config, flow);
            reconciled += 1;
        }
    }
    tracing::debug!(target: "flows", scanned = flows.len(), reconciled, skipped, "[flows] boot reconciliation of schedule-trigger cron jobs complete");
    Ok(())
}

/// Reads a settled run's durable [`tinyflows::engine::GraphObservation`]
/// slice back out of the per-run journal (keyed by the tinyagents-minted
/// `graph_run_id`) and exports it to Langfuse as one trace. Best-effort by
/// construction: any journal read failure is logged and swallowed, and the
/// exporter itself never fails the run. Skips the journal read entirely when
/// `observability.share_usage_data` is off.
async fn export_run_to_langfuse(
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
    crate::openhuman::flows::tinyflows::langfuse_export::export_flow_run_trace(
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

/// Runs a saved flow end-to-end: compile → build capabilities → durable
/// checkpointed run → record the outcome onto the flow's summary fields and
/// into a `flow_runs` history row.
///
/// Uses `tinyflows::engine::run_with_checkpointer` (not the simpler `run`) so
/// a run that pauses at a human-in-the-loop approval gate is durably
/// checkpointed and can survive a process restart (resumed later via
/// [`flows_resume`]; see
/// `my_docs/ohxtf/b1-engine-seam-domain/05-checkpointer-and-state.md`).
///
/// The whole run is scoped under `AgentTurnOrigin::TrustedAutomation {
/// Workflow }` (issue B2) regardless of caller (an interactive RPC "Run" or
/// an automatic trigger dispatch from `flows::bus::FlowTriggerSubscriber`):
/// the trust argument is about the *flow* (a saved, validated graph whose
/// `tool_call`/`http_request` nodes are pre-declared), not about who started
/// the run — see `TrustedAutomationSource::Workflow`'s doc and
/// `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §3.
/// `input` is the free-form trigger payload (reachable as `=run.trigger.…`);
/// `inputs` supplies values for the flow's *declared* workflow inputs by name
/// (reachable as `=inputs.<name>`). The two are separate channels — see
/// [`tinyflows::engine::RunInput`]. A declared-input problem (missing required
/// value, wrong type, undeclared key) is rejected before any run row exists.
pub async fn flows_run(
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<RpcOutcome<Value>, String> {
    // Prep synchronously (validate + compile-check + resolve inputs + mint the
    // run id), insert the initial `running` row, and announce it, then hand off
    // to the shared run body. Both the synchronous "Run" RPC path (this fn) and
    // the detached agent path ([`flows_run_detached`]) reuse `run_flow_body` so
    // a single [`RunRowFinalizer`] guards the row on every exit — bug B42.
    let prepared = prepare_flow_run(config, flow_id, &inputs)?;
    let thread_id = prepared.thread_id.clone();
    let no_actionable_nodes = prepared.no_actionable_nodes;
    let resolved_inputs = prepared.inputs;

    // Register BEFORE the row exists, so a `flows_cancel_run` can never observe
    // a `running` row that no live run owns (see [`run_flow_body`]'s doc).
    let (cancel_token, run_guard) = run_registry::register(&thread_id);
    start_flow_run_row(config, &thread_id, flow_id);
    publish_flow_run_started(flow_id, &thread_id);

    run_flow_body(
        Arc::new(config.clone()),
        prepared.flow,
        flow_id.to_string(),
        thread_id,
        input,
        resolved_inputs,
        trigger,
        no_actionable_nodes,
        cancel_token,
        run_guard,
    )
    .await
}

/// Agent-initiated `run_flow` entry point (bug B41). Unlike [`flows_run`], this
/// does NOT block on the engine: the tinyagents harness caps a single tool call
/// at 120s, but any flow whose first real node is a live-research agent node
/// (`web_search` + `web_fetch` + `parallel_research`) inherently runs longer
/// than that, so a blocking `run_flow` tool call could *never* succeed for a
/// realistic flow — it died at exactly 120s, orphaning the run row (bug B42).
///
/// Instead this validates + compile-checks the flow synchronously (so a broken
/// flow still returns an immediate, actionable error to the agent), inserts the
/// `running` row, publishes `FlowRunStarted`, then spawns [`run_flow_body`] on a
/// background task and returns `{ run_id, status: "running", detached: true }`
/// in well under 120s. The copilot already polls `get_flow_run(run_id)` (seen
/// in live traces), so it observes the run settle to a terminal state on its
/// own cadence. Also exposed over RPC as `flows.run_detached` (see
/// `schemas::handle_run_detached`) — the UI "Run" control (canvas + Workflows
/// list) calls that entry point directly, and the trigger bus
/// (`flows::bus::spawn_run`) fires runs the same fire-and-forget way. Combined
/// with B42's finalizer + boot sweep, a detached run ALWAYS settles to a
/// terminal row even if the process dies mid-run.
///
/// `input` / `inputs` mean exactly what they do on [`flows_run`]: the trigger
/// payload and the flow's declared inputs. Both are validated synchronously, so
/// the agent still gets an immediate, actionable error for a bad call.
pub async fn flows_run_detached(
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<RpcOutcome<Value>, String> {
    let prepared = prepare_flow_run(config, flow_id, &inputs)?;
    let thread_id = prepared.thread_id.clone();
    let no_actionable_nodes = prepared.no_actionable_nodes;
    let resolved_inputs = prepared.inputs;

    // Register BEFORE the `run_id` becomes observable to the agent. The spawned
    // task below may not be polled for some time, so registering inside it
    // would leave a window where a `flows_cancel_run` on the returned `run_id`
    // sees no in-flight run, settles the row `cancelled` + drops the
    // checkpoint, and the background run then executes the flow's real side
    // effects anyway and overwrites that terminal status. Registering here
    // means such a cancel always takes the signalled branch and this run's own
    // cancellation arm unwinds it. See [`run_flow_body`]'s doc.
    let (cancel_token, run_guard) = run_registry::register(&thread_id);
    start_flow_run_row(config, &thread_id, flow_id);
    publish_flow_run_started(flow_id, &thread_id);

    tracing::info!(
        target: "flows",
        flow_id = %flow_id,
        run_id = %thread_id,
        "[flows] flows_run_detached: registered + spawning background run; returning run_id immediately"
    );

    let config_arc = Arc::new(config.clone());
    let flow = prepared.flow;
    let flow_id_owned = flow_id.to_string();
    let body_thread_id = thread_id.clone();
    tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async move {
        if let Err(e) = run_flow_body(
            config_arc,
            flow,
            flow_id_owned,
            body_thread_id,
            input,
            resolved_inputs,
            trigger,
            no_actionable_nodes,
            cancel_token,
            run_guard,
        )
        .await
        {
            // The row is already reconciled by the body's terminal write /
            // finalizer — this only logs that the detached run ended in error.
            tracing::warn!(target: "flows", error = %e, "[flows] flows_run_detached: background run ended with error (row already reconciled)");
        }
        },
    ));

    let result = json!({
        "run_id": thread_id,
        "flow_id": flow_id,
        "status": "running",
        "detached": true,
    });
    Ok(RpcOutcome::single_log(
        result,
        format!("flow run started (detached): {thread_id}"),
    ))
}

/// A validated, ready-to-execute flow run: the loaded [`Flow`], the freshly
/// minted `thread_id` (== run id / checkpointer key), and whether the graph has
/// no actionable nodes. Produced by [`prepare_flow_run`] and consumed by both
/// `flows_run` entry points.
struct PreparedFlowRun {
    flow: Flow,
    thread_id: String,
    no_actionable_nodes: bool,
    /// The flow's declared inputs resolved against the caller's values —
    /// defaults applied, one entry per declaration.
    inputs: serde_json::Map<String, Value>,
}

/// Synchronous prep shared by [`flows_run`] and [`flows_run_detached`]: loads
/// the flow, warns on an actionless graph, rejects an engine-incompatible
/// topology, compile-checks the graph so a broken flow fails fast *before* any
/// `running` row is inserted, resolves the caller's declared-input values, and
/// mints the run's `thread_id`. Returns an error (never a wedged row) if the
/// flow can't run at all.
///
/// Input resolution happens *here* rather than being left to the engine so a
/// bad call never creates a `running` row, a thread id, or a registry entry.
/// The engine re-resolves the same values (it is the authority on its own
/// contract); doing it twice is cheap and keeps this host from having to trust
/// its own copy of the rules.
fn prepare_flow_run(
    config: &Config,
    flow_id: &str,
    inputs: &serde_json::Map<String, Value>,
) -> Result<PreparedFlowRun, String> {
    let flow = store::get_flow(config, flow_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{flow_id}' not found"))?;

    // Live finding: a graph with no actionable nodes (only a `trigger`, or a
    // `trigger` plus nodes with no edges wiring them up) compiles and "runs"
    // cleanly but does nothing — and previously reported
    // `status="completed" pending_approvals=0` indistinguishably from a real
    // run, reading as "triggered but nothing happened" was actually a
    // success. Surface it loudly instead of letting it pass silently: warn
    // now (independent of how the run below turns out), and attach a
    // human-readable note to the returned outcome so the UI can show
    // "nothing to run" rather than a bare "completed".
    let no_actionable_nodes = !graph_has_actionable_nodes(&flow.graph);
    if no_actionable_nodes {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            "[flows] flows_run: flow has no actionable nodes — nothing to execute"
        );
    }

    // `store::get_flow` already ran the stored `graph_json` through
    // `tinyflows::migrate::migrate` before deserializing, so `flow.graph` is
    // always on the current schema here.
    //
    // Author-time validation cannot protect definitions persisted by an older
    // OpenHuman build. Re-check immediately before compilation so an upgrade
    // fails explicitly instead of silently committing incomplete merge data.
    if let Err(error) = ensure_config_aware_engine_compatible(config, &flow.graph) {
        tracing::warn!(
            target: "flows",
            flow_id = %flow_id,
            %error,
            "[flows] flows_run: rejected — unsupported engine topology"
        );
        return Err(error);
    }
    // Compile-check up front so a structurally broken graph fails the caller
    // immediately, before a `running` row exists. `run_flow_body` recompiles
    // (cheap) to actually execute.
    tinyflows::compiler::compile(&flow.graph).map_err(|e| e.to_string())?;

    // Declared inputs, before anything observable exists for this run.
    let resolved_inputs =
        tinyflows::model::resolve_inputs(&flow.graph.inputs, inputs).map_err(|e| {
            tracing::warn!(
                target: "flows",
                flow_id = %flow_id,
                input = %e.input_name(),
                code = %e.code(),
                "[flows] flows_run: rejected — bad workflow input"
            );
            e.to_string()
        })?;

    let thread_id = format!("flow:{flow_id}:{}", uuid::Uuid::new_v4());
    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        thread_id = %thread_id,
        require_approval = flow.require_approval,
        "[flows] flows_run: prepared checkpointed run"
    );

    Ok(PreparedFlowRun {
        flow,
        thread_id,
        no_actionable_nodes,
        inputs: resolved_inputs,
    })
}

/// Announces a freshly-started run on the global event bus so the frontend run
/// list flips to `running` immediately. Factored out of [`flows_run`] so both
/// entry points publish identically.
fn publish_flow_run_started(flow_id: &str, thread_id: &str) {
    tracing::debug!(
        target: "flows",
        flow_id = %flow_id,
        run_id = %thread_id,
        "[flows] flows_run: publishing FlowRunStarted"
    );
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::FlowRunStarted {
        flow_id: flow_id.to_string(),
        run_id: thread_id.to_string(),
    });
}

/// Human-readable reason stamped on a run row that the [`RunRowFinalizer`]
/// drop-guard reconciles because its run future was dropped mid-flight (harness
/// tool abort, chat turn end, runtime shutdown, panic) before any terminal
/// write landed. Surfaced verbatim in the run-details sidebar (bug B42c) so a
/// cancelled/timed-out run reads as interrupted rather than a blank spinner.
const INTERRUPTED_DROP_REASON: &str =
    "Run interrupted before completion — it was cancelled, timed out, or the app shut down mid-run.";

/// Cancellation-safe finalizer for a live `flow_runs` row (bug B42).
///
/// While a run's engine future is awaiting, dropping that future — the harness
/// 120s tool abort, a chat turn ending, tokio runtime shutdown, or a panic —
/// would otherwise leave the row wedged at `status="running"`, `error=NULL`,
/// `steps=[]` forever, which the run-details sidebar renders as a perpetual
/// blank spinner. Held across the await, this guard writes a terminal
/// `"interrupted"` status + human reason on `Drop` UNLESS it has been
/// explicitly [`disarm`](Self::disarm)ed after a real terminal write. The
/// `armed` flag is a single-task `Cell` (the guard never crosses tasks by
/// reference), so the type stays `Send` for `tokio::spawn`.
struct RunRowFinalizer {
    config: Arc<Config>,
    thread_id: String,
    flow_id: String,
    armed: std::cell::Cell<bool>,
}

impl RunRowFinalizer {
    fn new(config: Arc<Config>, thread_id: &str, flow_id: &str) -> Self {
        Self {
            config,
            thread_id: thread_id.to_string(),
            flow_id: flow_id.to_string(),
            armed: std::cell::Cell::new(true),
        }
    }

    /// Disarm the guard after a real terminal write (success/failure/cancel/
    /// pause) has already finalized the row, so `Drop` becomes a no-op.
    fn disarm(&self) {
        self.armed.set(false);
    }
}

impl Drop for RunRowFinalizer {
    fn drop(&mut self) {
        if !self.armed.get() {
            return;
        }
        tracing::warn!(
            target: "flows",
            flow_id = %self.flow_id,
            thread_id = %self.thread_id,
            "[flows] RunRowFinalizer: run future dropped before settling — reconciling orphaned 'running' row to 'interrupted'"
        );
        // Preserve whatever steps the live observer already persisted.
        let observed = current_persisted_steps(&self.config, &self.thread_id);
        finish_flow_run_row(
            &self.config,
            &self.thread_id,
            &self.flow_id,
            "interrupted",
            &observed,
            &[],
            Some(INTERRUPTED_DROP_REASON),
            None,
        );
        // Keep the flow-definition summary in step with the row, exactly as the
        // success/failure/cancel arms and the boot sweep do — otherwise the
        // runs list keeps advertising the *previous* run's `last_status` /
        // `last_run_at` for a flow whose latest run was interrupted.
        // `record_run` is synchronous, so it is safe in `Drop`.
        if let Err(e) = store::record_run(&self.config, &self.flow_id, "interrupted") {
            tracing::warn!(
                target: "flows",
                flow_id = %self.flow_id,
                thread_id = %self.thread_id,
                error = %e,
                "[flows] RunRowFinalizer: failed to update flow summary for interrupted run"
            );
        }
    }
}
