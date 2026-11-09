use super::*;
use crate::core::runtime::DispatchCtx;

/// The single canonical definition of the builder hard-gate stack: the
/// author-time gates that reject (not warn) a graph an agent must not propose
/// or persist — engine compatibility, binding-resolvability, agent-ref
/// resolvability, connection-ref, tool-contract, and required-arg
/// resolvability, in increasing cost order.
///
/// Returns an empty `Vec` when the graph passes; otherwise the first failing
/// gate's node-level error messages (short-circuiting, so an expensive later
/// gate never runs on a graph already known to be broken). Every plane that
/// gates an agent-authored graph — `build_builder_proposal` (propose / revise /
/// edit), `save_workflow`, and the `strict` create/update RPC path — routes
/// through here, so they cannot drift (audit F3: agent saves and UI saves used
/// to validate differently).
///
/// Assumes `graph` is already structurally valid (run
/// `validate_and_migrate_graph` / `validate_all` first) — these gates check
/// resolvability/contracts on a compilable graph.
///
pub(crate) async fn run_builder_gates(
    ctx: &DispatchCtx,
    config: &Config,
    graph: &Value,
) -> Vec<String> {
    // The stack order, short-circuiting and mock-run preflight are
    // `tinyflows::gates::builder_failures`'; the host supplies the stages that
    // depend on its vocabulary (agent registry, connections, tool catalog).
    //
    // Provider connectivity (B45) is deliberately NOT a stage: it is an advisory
    // `inference_status` on the proposal payload and a run-time rejection, so a
    // graph that is only "not runnable yet" can still be shown to the user.
    let host = match crate::flows::authoring_host::register(
        std::sync::Arc::new(config.clone()),
        ctx.clone(),
    )
    .await
    {
        Ok(host) => host,
        Err(error) => return vec![format!("builder gate could not complete: {error}")],
    };
    let result = async {
        let proxy = crate::modules::tinyflows::authoring_proxy(config).await?;
        proxy
            .call::<Vec<String>>(
                tinyflows_bus::methods::BUILDER_FAILURES,
                &(
                    graph.clone(),
                    host.authoring.clone(),
                    vec![crate::flows::caps_host::NATIVE_TOOL_PREFIX.to_string()],
                ),
            )
            .await
            .map_err(|error| error.wire_message())
    }
    .await;
    match result {
        Ok(failures) => failures,
        Err(error) => {
            tracing::warn!(
                target: "flows",
                %error,
                "[flows] builder gates: host stage failed — reporting it as a gate failure"
            );
            vec![format!("builder gate could not complete: {error}")]
        }
    }
}

/// Strict-mode gate for the create/update RPC path (audit F3): validates
/// `graph_json` structurally (surfacing every error at once) and then runs the
/// same [`run_builder_gates`] the agent tools enforce, returning `Err` with a
/// combined, model-consumable message if anything fails.
///
/// The UI/RPC create/update path stays permissive by default (a human editing
/// on the canvas may save a work-in-progress graph); passing `strict: true`
/// opts that call into the *same* gates an agent save must pass, so the two
/// planes converge on one definition instead of diverging.
pub(crate) async fn strict_gate(
    ctx: &DispatchCtx,
    config: &Config,
    graph_json: &Value,
) -> Result<(), String> {
    let graph = validate_and_migrate_graph(ctx, config, graph_json.clone())
        .await
        .map_err(|error| {
            format!("strict validation failed — the graph is structurally invalid:\n{error}")
        })?;
    let gate_errors = run_builder_gates(ctx, config, &graph).await;
    if !gate_errors.is_empty() {
        return Err(format!(
            "strict validation failed:\n{}",
            gate_errors.join("\n\n")
        ));
    }
    Ok(())
}

/// Runs the full builder hard-gate stack on an already structurally-valid
/// `graph` and, if it passes, builds the `workflow_proposal` payload the
/// propose/revise/edit tools all return.
///
/// The single home for the gate sequence (`tinyflows::gates::builder_failures`) plus
/// summary/warning assembly,
/// so `revise_workflow` and `edit_workflow` cannot drift. `retry_tool` names
/// the tool in the "fix … and call `<tool>` again" guidance so each caller's
/// error text points the agent back at the right tool.
///
/// `draft_id` / `flow_id` are OPTIONAL persistence-state context echoed onto
/// the payload (the draft this proposal's edit lives on, and the saved flow it
/// derives from / targets). The payload ALWAYS carries `"persisted": false` so
/// a proposal can never be mistaken for a save confirmation — the exact false
/// belief the WS2 audit caught (an agent read a proposal as "written onto the
/// saved flow"). Actual persistence only happens via `save_workflow` /
/// `create_workflow` / `flows_draft_promote`.
///
/// Returns `Ok(payload)` on success, or `Err(message)` with a
/// model-consumable, fix-and-retry error when a gate rejects the graph. The
/// caller is responsible for structural validation (`validate_and_migrate_graph`
/// / `validate_all`) *before* calling this — these gates assume a compilable
/// graph.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_builder_proposal(
    ctx: &DispatchCtx,
    config: &Config,
    retry_tool: &str,
    name: &str,
    graph: &Value,
    require_approval: bool,
    revision: bool,
    instruction: Option<String>,
    draft_id: Option<String>,
    flow_id: Option<String>,
) -> Result<Value, String> {
    // The full builder hard-gate stack, run through the single canonical
    // runner so every proposal/save/strict-RPC path gates identically (F3).
    let gate_errors = run_builder_gates(ctx, config, graph).await;
    if !gate_errors.is_empty() {
        return Err(format!(
            "{}\n\nFix these and call {retry_tool} again.",
            gate_errors.join("\n\n")
        ));
    }

    let analysis = super::validation::validate_over_bus(ctx, config, graph.clone()).await?;
    let summary = analysis
        .summary
        .ok_or_else(|| "TinyFlows validation returned no graph summary".to_string())?;
    let mut warnings = analysis.warnings;
    match wiring_warnings_json(ctx, config, graph.clone()).await {
        Ok(wiring_warnings) => warnings.extend(wiring_warnings),
        Err(error) => {
            tracing::warn!(target: "flows", %error, "[flows] TinyFlows wiring warnings failed")
        }
    }
    // Connector onboarding (Phase 5, item 18): tell the proposal card which
    // toolkits this graph needs and whether they're connected, so it can render
    // "Connect <toolkit>" CTAs instead of a bare gate error later.
    let required_connections = compute_required_connections(ctx, config, graph).await;
    // B45 (design correction): the LLM-provider-connectivity evaluation is
    // ADVISORY here, never a rejection — `run_builder_gates` above no longer
    // includes it (that used to hard-block `propose_workflow`/`edit_workflow`
    // on a graph the copilot couldn't then show the user at all — judge
    // finding on live run 104aab90). So `evaluation.status` here can
    // legitimately be `"ready"`, `"signed_out"`, `"provider_not_configured"`,
    // or `"error"` — the UI renders a "Connect a provider" / "Sign in" CTA
    // for the non-ready cases, alongside the toolkit-connection CTAs above.
    // The graph is proposed regardless of this value. Computed via the same
    // shared, cached evaluator the run-time preflight (`validate_inference_readiness`
    // in `run_flow_body`) consumes, so a run right after this proposal reads
    // the cached result instead of re-probing the network.
    let inference_readiness = evaluate_inference_readiness(ctx, config, graph).await;
    // The proposal's name is the one the builder resolved (its `name` argument,
    // else the base graph's). A graph without its own name carried "" into the
    // card and onto a save, so stamp the resolved name into the graph too.
    let mut graph_value = graph.clone();
    if let Some(graph_object) = graph_value.as_object_mut() {
        graph_object.insert("name".to_string(), json!(name));
    }

    tracing::info!(
        target: "flows",
        %name,
        node_count = graph
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len),
        require_approval,
        warning_count = warnings.len(),
        revision,
        "[flows] build_builder_proposal: proposal ready for user review"
    );

    let mut payload = json!({
        "type": "workflow_proposal",
        "revision": revision,
        // A proposal is NEVER a persisted flow — it is a candidate the user
        // still has to accept/save. Stamp this unconditionally so the payload
        // can't be misread as a save confirmation (WS2 audit).
        "persisted": false,
        "name": name,
        "graph": graph_value,
        "require_approval": require_approval,
        "summary": summary,
        "warnings": warnings,
        "required_connections": required_connections,
    });
    // Only present when the graph has at least one applicable `agent` node;
    // a tool_call-only graph omits both fields entirely rather than claiming
    // a meaningless "ready".
    if let Some(evaluation) = inference_readiness {
        payload["inference_status"] = json!(evaluation.status);
        if let Some(message) = evaluation.message {
            payload["inference_message"] = json!(message);
        }
    }
    if let Some(instruction) = instruction {
        payload["instruction"] = json!(instruction);
    }
    // Echo the persistence-state handles so the agent can iterate/persist
    // against the right ids (the draft the edit lives on; the flow it targets).
    if let Some(flow_id) = flow_id.as_deref() {
        // The version this revision was derived from, so a save onto the flow
        // can refuse to overwrite a newer edit (`flows_update`'s
        // `expected_version`). Omitted when it cannot be established; the card
        // then asks for a new revision instead of saving last-write-wins.
        if let Some(version) = source_flow_version(config, flow_id, draft_id.as_deref()) {
            payload["flow_version"] = json!(version);
        }
    }
    if let Some(draft_id) = draft_id {
        payload["draft_id"] = json!(draft_id);
    }
    if let Some(flow_id) = flow_id {
        payload["flow_id"] = json!(flow_id);
    }
    Ok(payload)
}

/// The version of `flow_id` a revision's graph was derived from.
///
/// A revision of a saved flow lives on a draft seeded from that flow, and the
/// draft records no base version. The flow's current `updated_at` is the
/// source version exactly when the flow has not changed since the draft was
/// created; after a later edit it is not, and a version read now would let a
/// stale graph pass `expected_version`. `None` when that cannot be shown.
fn source_flow_version(config: &Config, flow_id: &str, draft_id: Option<&str>) -> Option<String> {
    let flow = match store::get_flow(config, flow_id) {
        Ok(Some(flow)) => flow,
        Ok(None) => {
            tracing::debug!(target: "flows", %flow_id, "[flows] source_flow_version: target flow not found");
            return None;
        }
        Err(error) => {
            tracing::warn!(target: "flows", %flow_id, %error, "[flows] source_flow_version: could not read target flow");
            return None;
        }
    };
    let Some(draft_id) = draft_id else {
        tracing::debug!(target: "flows", %flow_id, "[flows] source_flow_version: no draft to date the revision");
        return None;
    };
    let draft = match draft_store::get_draft(config, draft_id) {
        Ok(Some(draft)) => draft,
        Ok(None) => {
            tracing::debug!(target: "flows", %draft_id, "[flows] source_flow_version: draft not found");
            return None;
        }
        Err(error) => {
            tracing::warn!(target: "flows", %draft_id, %error, "[flows] source_flow_version: could not read draft");
            return None;
        }
    };
    let flow_at = chrono::DateTime::parse_from_rfc3339(&flow.updated_at).ok()?;
    let seeded_at = chrono::DateTime::parse_from_rfc3339(&draft.created_at).ok()?;
    if flow_at <= seeded_at {
        Some(flow.updated_at)
    } else {
        tracing::debug!(
            target: "flows",
            %flow_id,
            %draft_id,
            "[flows] source_flow_version: flow changed after the draft was seeded; no version"
        );
        None
    }
}

/// Decode the Copilot bus's portable graph payload and run the host-owned
/// proposal gates without exposing engine types through the callback adapter.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn build_builder_proposal_json(
    ctx: &DispatchCtx,
    config: &Config,
    retry_tool: String,
    name: String,
    graph: Value,
    require_approval: bool,
    revision: bool,
    instruction: Option<String>,
    draft_id: Option<String>,
    flow_id: Option<String>,
) -> Result<Value, String> {
    let graph = validate_and_migrate_graph(ctx, config, graph).await?;
    build_builder_proposal(
        ctx,
        config,
        &retry_tool,
        &name,
        &graph,
        require_approval,
        revision,
        instruction,
        draft_id,
        flow_id,
    )
    .await
}

/// Run the configured builder gates on the Copilot bus's portable graph payload.
pub(crate) async fn run_builder_gates_json(
    ctx: &DispatchCtx,
    config: &Config,
    graph: Value,
) -> Result<Vec<String>, String> {
    let graph = validate_and_migrate_graph(ctx, config, graph).await?;
    Ok(run_builder_gates(ctx, config, &graph).await)
}

// ─────────────────────────────────────────────────────────────────────────────
