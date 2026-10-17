use super::*;

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
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<Outcome<Value>, String> {
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
        ctx.clone(),
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
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<Outcome<Value>, String> {
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
    let body_ctx = ctx.clone();
    let flow = prepared.flow;
    let flow_id_owned = flow_id.to_string();
    let body_thread_id = thread_id.clone();
    tokio::spawn(crate::core::runtime::context::CoreContext::propagate(
        async move {
            if let Err(e) = run_flow_body(
                body_ctx,
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
    Ok(Outcome::single_log(
        result,
        format!("flow run started (detached): {thread_id}"),
    ))
}

/// A validated, ready-to-execute flow run: the loaded [`Flow`], the freshly
/// minted `thread_id` (== run id / checkpointer key), and whether the graph has
/// no actionable nodes. Produced by [`prepare_flow_run`] and consumed by both
/// `flows_run` entry points.
pub(super) struct PreparedFlowRun {
    pub(super) flow: Flow,
    pub(super) thread_id: String,
    pub(super) no_actionable_nodes: bool,
    /// The flow's declared inputs resolved against the caller's values —
    /// defaults applied, one entry per declaration.
    pub(super) inputs: serde_json::Map<String, Value>,
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
pub(super) fn prepare_flow_run(
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
