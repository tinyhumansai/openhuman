use super::*;

/// Publishes a [`DomainEvent::FlowChanged`](crate::core::events::DomainEvent::FlowChanged)
/// so an open Workflows list/canvas refetches (bridged to a `flow:changed`
/// socket event) — the observability half of audit F6. Best-effort broadcast;
/// `actor` is a coarse hint (`"system"` for RPC-driven changes today).
pub(super) fn publish_flow_changed(flow_id: &str, kind: &str, actor: &str) {
    tracing::debug!(target: "flows", %flow_id, kind, actor, "[flows] publishing FlowChanged");
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::FlowChanged {
        flow_id: flow_id.to_string(),
        kind: kind.to_string(),
        actor: actor.to_string(),
    });
}

/// Creates a new flow from a name and a raw graph JSON value.
///
/// Catalog.PutWorkflow validates and persists the graph, applies automatic-
/// trigger disarm and outbound-action approval policy, and returns its
/// authoritative `born_disabled` / `approval_forced` results for the host RPC
/// logs. OpenHuman binds the automatic-dispatch trigger only when the saved
/// result is enabled.
pub async fn flows_create(
    config: &Config,
    name: String,
    graph_json: Value,
    require_approval: bool,
) -> Result<Outcome<Flow>, String> {
    flows_create_with_context(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        name,
        graph_json,
        require_approval,
    )
    .await
}

pub(crate) async fn flows_create_with_context(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    name: String,
    graph_json: Value,
    require_approval: bool,
) -> Result<Outcome<Flow>, String> {
    ctx.enter(async move {
        let graph = validate_and_migrate_graph(ctx, config, graph_json).await?;
        ensure_config_aware_engine_compatible(ctx, config, &graph).await?;

    // Rule 1: automatic triggers create DISABLED — the user must arm them
    // explicitly.
    tracing::debug!(
        target: "flows",
        %name,
        node_count = graph
            .get("nodes")
            .and_then(serde_json::Value::as_array)
            .map_or(0, Vec::len),
        require_approval,
        "[flows] flows_create: persisting new flow"
    );
    let result = store::create_flow(config, name, graph, require_approval, true)
        .map_err(|e| e.to_string())?;
    let flow = result.workflow;
    let born_disabled = result.born_disabled;
    let approval_forced = result.approval_forced;

    if flow.enabled {
        tracing::debug!(target: "flows", flow_id = %flow.id, "[flows] flows_create: flow is enabled — binding automatic-dispatch trigger");
        bind_trigger(config, &flow);
    }

    let mut logs = vec!["flow created".to_string()];
    if born_disabled {
        let trigger_label = match bus::describe_trigger(config, &flow) {
            Ok(details) => details
                .kind
                .unwrap_or_else(|| "automatic".to_string()),
            Err(error) => {
                tracing::warn!(target: "flows", flow_id = %flow.id, %error, "[flows] could not describe trigger after create");
                "automatic".to_string()
            }
        };
        logs.push(format!(
            "Flow created DISABLED because it has an automatic trigger ({trigger_label}). \
             Enable it explicitly (flows_set_enabled) when you are ready for it to fire."
        ));
    }
    if approval_forced {
        logs.push(
            "require_approval forced to true because the graph contains outbound side-effect \
             nodes (tool_call / http_request / code)."
                .to_string(),
        );
    }

        publish_flow_changed(&flow.id, "created", "system");
        Ok(Outcome::new(flow, logs))
    })
    .await
}

/// Duplicates a saved flow: creates an independent copy of its graph under a
/// new id/timestamps, with the name suffixed `" (copy)"`. The copy is created
/// **disabled** (`enabled = false`) and therefore **not** schedule/app_event
/// trigger-bound — unlike [`flows_create`], which binds a trigger for an
/// enabled flow, this deliberately calls no [`bind_trigger`], so a duplicate
/// can never immediately fire. Run history does not carry over. The user
/// enables it explicitly (via `flows_set_enabled`) once they've reviewed the
/// copy, at which point its trigger binds like any other flow.
pub async fn flows_duplicate(config: &Config, id: &str) -> Result<Outcome<Flow>, String> {
    let source = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    let new_name = format!("{} (copy)", source.name);
    tracing::debug!(target: "flows", source_id = %id, %new_name, "[flows] flows_duplicate: creating disabled, unbound copy");
    let flow =
        store::insert_duplicate_flow(config, &source, new_name).map_err(|e| e.to_string())?;
    // Intentionally NO bind_trigger: a duplicate is disabled and must stay
    // inert (no schedule/trigger dispatch) until the user enables it.
    publish_flow_changed(&flow.id, "created", "system");
    Ok(Outcome::single_log(
        flow,
        format!("flow duplicated from {id}"),
    ))
}

/// Loads one flow by id.
pub async fn flows_get(config: &Config, id: &str) -> Result<Outcome<Flow>, String> {
    let flow = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;
    Ok(Outcome::single_log(flow, format!("flow loaded: {id}")))
}

/// Loads a saved flow's portable JSON graph by id, for the
/// `sub_workflow`-by-`workflow_id` resolver capability through the compiled
/// TinyFlows module's Catalog contract.
///
/// Returns `Ok(None)` when no flow with that id exists (the resolver turns that
/// into a capability error naming the missing id), and `Err` only on a store
/// failure. Kept sync because the underlying [`store::get_flow`] is sync.
pub fn load_flow_graph(config: &Config, id: &str) -> Result<Option<Value>, String> {
    tracing::debug!(target: "flows", flow_id = %id, "[flows] load_flow_graph: loading saved flow graph for sub_workflow resolver");
    let graph = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .map(|flow| flow.graph);
    tracing::debug!(
        target: "flows",
        flow_id = %id,
        found = graph.is_some(),
        "[flows] load_flow_graph: resolver lookup complete"
    );
    Ok(graph)
}

/// Resolver-only saved-graph lookup. Authoring tools use [`load_flow_graph`]
/// so a legacy draft can still be opened and repaired; execution resolves only
/// graphs the current engine can run safely.
pub(crate) async fn load_engine_compatible_flow_graph(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    id: &str,
) -> Result<Option<Value>, String> {
    let graph = load_flow_graph(config, id)?;
    if let Some(graph) = graph.as_ref() {
        ensure_config_aware_engine_compatible(ctx, config, graph)
            .await
            .map_err(|error| format!("workflow_id '{id}' is engine-incompatible: {error}"))?;
    }
    Ok(graph)
}

/// Lists every saved flow.
///
/// A corrupt or newer-schema-than-this-build `graph_json` row is skipped
/// rather than failing the whole list (R-M4 — see `store::list_flow_rows`);
/// when that happens it must not be silent, so a skip is both logged
/// (`[flows]`-prefixed, id + error only — never row content) and surfaced in
/// the RPC's `logs` so the UI can tell the user "N workflows could not be
/// loaded" instead of silently rendering a shorter list than actually exists.
pub async fn flows_list(config: &Config) -> Result<Outcome<Vec<Flow>>, String> {
    let (flows, skipped) = store::list_flows(config).map_err(|e| e.to_string())?;
    if skipped > 0 {
        tracing::warn!(
            target: "flows",
            skipped,
            loaded = flows.len(),
            "[flows] flows_list: skipped corrupt/unmigratable flow_definitions rows"
        );
        Ok(Outcome::new(
            flows,
            vec![format!(
                "flows listed ({skipped} workflow{} could not be loaded and were skipped)",
                if skipped == 1 { "" } else { "s" }
            )],
        ))
    } else {
        Ok(Outcome::single_log(flows, "flows listed"))
    }
}

/// Deletes a flow by id.
///
/// Removes the flow definition before unbinding its separately persisted
/// automatic-dispatch trigger (e.g. the schedule-trigger job). Removing the
/// flow first means a failed delete leaves its still-existing schedule intact.
/// Best-effort: a lookup failure is logged and does not block the delete.
pub async fn flows_delete(config: &Config, id: &str) -> Result<Outcome<Value>, String> {
    flows_delete_with_remove(config, id, |config, id| {
        store::remove_flow(config, id).map_err(|e| e.to_string())
    })
    .await
}

/// Delete implementation split at the fallible store operation so failure
/// ordering can be exercised without corrupting the configured storage.
pub(super) async fn flows_delete_with_remove(
    config: &Config,
    id: &str,
    remove: impl FnOnce(&Config, &str) -> Result<(), String>,
) -> Result<Outcome<Value>, String> {
    let flow = match store::get_flow(config, id) {
        Ok(Some(flow)) => Some(flow),
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %id, error = %e, "[flows] flows_delete: failed to load flow before unbind — proceeding with delete anyway");
            None
        }
    };

    remove(config, id)?;
    if let Some(flow) = flow {
        unbind_trigger(config, &flow);
    }
    tracing::debug!(target: "flows", flow_id = %id, "[flows] flows_delete: removed");

    // Best-effort: purge the flow's pre-authorized tool trust with its row —
    // a deleted flow must not leave dangling `flow_tool_trust` grants that a
    // future flow reusing the same id (or a stale run) could inherit. Never
    // fails the delete: the flow row is already gone regardless.
    if let Some(gate) = crate::security::approval::ApprovalGate::try_global() {
        match gate.delete_flow_trust(id, None) {
            Ok(removed) if removed > 0 => {
                tracing::info!(target: "flows", flow_id = %id, removed, "[flows] flows_delete: purged flow tool trust grants");
            }
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(target: "flows", flow_id = %id, error = %e, "[flows] flows_delete: failed to purge flow tool trust");
            }
        }
    }

    // Best-effort: forget this flow's memory along with its row — a deleted
    // flow must not leave stray `flow_memory_remember` items or run digests
    // behind. Memory off forgets nothing. Never fails the delete itself: the
    // flow row is already gone by this point.
    match crate::flows::forget_matching(config, crate::flows::flow_filter(id)).await {
        Ok(forgotten) => {
            tracing::debug!(target: "flows", flow_id = %id, forgotten, "[flows] flows_delete: forgot flow memory");
        }
        Err(error) => {
            tracing::warn!(target: "flows", flow_id = %id, code = error.code(), %error, "[flows] flows_delete: failed to forget flow memory");
        }
    }

    publish_flow_changed(id, "deleted", "system");
    Ok(Outcome::new(
        json!({ "id": id, "removed": true }),
        vec![format!("flow removed: {id}")],
    ))
}
