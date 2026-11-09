use super::*;

/// Run a saved workflow and return the existing `{output,pending_approvals,
/// thread_id}` controller result after the TinyFlows module parks or settles
/// the run. The module validates inputs and graph compilation before it
/// creates any durable run state.
pub async fn flows_run(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<Outcome<Value>, String> {
    let prepared = prepare_flow_run(ctx, config, flow_id, &inputs).await?;
    let (session, mut events) = crate::flows::runs_adapter::start_run(
        ctx,
        config,
        &prepared.flow,
        &prepared.run_id,
        input,
        inputs,
        trigger,
    )
    .await?;
    let result =
        crate::flows::runs_adapter::wait_for_outcome(&mut events, &prepared.run_id, 0).await?;
    drop(session);
    Ok(run_outcome(result, prepared.no_actionable_nodes))
}

/// Start an agent-requested workflow without waiting for its engine result.
/// The RPC response remains the detached admission acknowledgement.
pub async fn flows_run_detached(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
    trigger: FlowRunTrigger,
) -> Result<Outcome<Value>, String> {
    let prepared = prepare_flow_run(ctx, config, flow_id, &inputs).await?;
    let (_session, _events) = crate::flows::runs_adapter::start_run(
        ctx,
        config,
        &prepared.flow,
        &prepared.run_id,
        input,
        inputs,
        trigger,
    )
    .await?;
    let result = json!({
        "run_id": prepared.run_id,
        "flow_id": flow_id,
        "status": "running",
        "detached": true,
    });
    Ok(Outcome::single_log(
        result,
        format!("flow run started (detached): {}", prepared.run_id),
    ))
}

/// Start the Copilot's confirmed test run as a detached RPC-origin flow run.
pub(crate) async fn flows_run_copilot(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    input: Value,
    inputs: serde_json::Map<String, Value>,
) -> Result<Outcome<Value>, String> {
    flows_run_detached(ctx, config, flow_id, input, inputs, FlowRunTrigger::Rpc).await
}

fn run_outcome(
    result: crate::flows::runs_adapter::RunOutcome,
    no_actionable_nodes: bool,
) -> Outcome<Value> {
    const NO_ACTIONABLE_NODES_NOTE: &str = "This flow's graph has no actionable nodes beyond \
         its trigger (no downstream action nodes, or no edges connecting them) — the run \
         completed without doing anything. Add and wire up at least one action node.";

    let status = result.status.clone();
    let mut value = json!({
        "output": result.output,
        "pending_approvals": result.pending_approvals,
        "thread_id": result.run_id,
    });
    let mut logs = vec![format!("flow run {status}")];
    if no_actionable_nodes {
        value["note"] = json!(NO_ACTIONABLE_NODES_NOTE);
        logs.push(NO_ACTIONABLE_NODES_NOTE.to_string());
    }
    Outcome::new(value, logs)
}

/// A validated host envelope for one upcoming Runs.StartRun call.
pub(super) struct PreparedFlowRun {
    pub(super) flow: Flow,
    pub(super) run_id: String,
    pub(super) no_actionable_nodes: bool,
}

/// Perform OpenHuman-owned checks before the module admits a run. Inference
/// readiness stays here so an unavailable provider fails before StartRun,
/// while graph compatibility, compilation, declared-input validation, row
/// creation, and execution remain module-owned.
pub(super) async fn prepare_flow_run(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    flow_id: &str,
    _inputs: &serde_json::Map<String, Value>,
) -> Result<PreparedFlowRun, String> {
    // Resolve the definition through the dispatch's captured CatalogStorage
    // scope. `store::get_flow` is a synchronous ambient-scope convenience API
    // and must not choose the acting identity for an explicitly dispatched run.
    let catalog = crate::flows::catalog_storage::register(ctx, config).await?;
    let proxy = crate::modules::tinyflows::catalog_proxy(config).await?;
    let workflow: Option<tinyflows_bus::catalog::Workflow> = proxy
        .call(
            tinyflows_bus::catalog::members::GET_WORKFLOW,
            (catalog.storage(), flow_id.to_owned()),
        )
        .await
        .map_err(|error| error.wire_message())?;
    let flow: Flow = workflow.ok_or_else(|| format!("flow '{flow_id}' not found"))?;
    let graph_json = flow.graph.clone();
    let analysis = super::validation::validate_over_bus(ctx, config, graph_json).await?;
    let no_actionable_nodes = !analysis
        .has_actionable_nodes
        .ok_or_else(|| "TinyFlows validation returned no actionability result".to_string())?;
    if no_actionable_nodes {
        tracing::warn!(
            target: "flows",
            flow_id,
            "[flows] flows_run: flow has no actionable nodes — nothing to execute"
        );
    }

    if let Err(error) = ensure_config_aware_engine_compatible(ctx, config, &flow.graph).await {
        tracing::warn!(
            target: "flows",
            flow_id,
            %error,
            "[flows] flows_run: rejected — unsupported engine topology"
        );
        return Err(error);
    }

    let inference_errors = validate_inference_readiness(ctx, config, &flow.graph).await;
    if !inference_errors.is_empty() {
        return Err(format!(
            "This flow's AI step needs a working AI provider to run. {}",
            inference_errors.join(" ")
        ));
    }

    let run_id = format!("flow:{flow_id}:{}", uuid::Uuid::new_v4());
    Ok(PreparedFlowRun {
        flow,
        run_id,
        no_actionable_nodes,
    })
}
