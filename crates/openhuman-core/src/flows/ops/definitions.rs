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
/// Issue B29 (save/enable safety) — two server-side rules apply here,
/// authoritative regardless of what the caller passed, so no creation path
/// (prompt bar, scratch/template modal, proposal "save & enable", copilot
/// `save_workflow`, …) can silently hand the user an armed, unattended
/// automation:
///
/// - **Rule 1** ([`trigger_is_automatic`]): a graph whose trigger fires
///   without a human in the loop (`schedule` / `app_event` / `webhook`)
///   persists **disabled**. The user arms it explicitly via
///   `flows_set_enabled` — the same toggle already used everywhere else. A
///   `manual` trigger (or no trigger-kind discriminator at all) still
///   persists enabled: it only ever runs via an explicit `flows_run`, so
///   there is no surprise, and gating it would just add friction.
///
///   This means a caller that represents an explicit user-arming action
///   (e.g. `WorkflowProposalCard`'s "Save & enable" click,
///   `app/src/components/chat/WorkflowProposalCard.tsx`) must check the
///   returned [`Flow`]'s `enabled` field and follow up with
///   `flows_set_enabled(id, true)` when it comes back `false` — otherwise
///   the button's own label lies to the user. That follow-up call is a
///   legitimate, explicit enable, not the silent copilot auto-arm this rule
///   exists to prevent (the copilot's `save_workflow` path has no such
///   follow-up and stays disabled).
/// - **Rule 2** ([`graph_has_outbound_side_effect`]): a graph containing any
///   `tool_call` / `http_request` / `code` node — the three kinds that can
///   produce a real outbound effect — forces `require_approval: true`,
///   overriding whatever the caller passed. A read-only graph (only
///   `trigger` / `agent` / `transform` / `condition` / data-flow nodes) is
///   unaffected.
///
/// An enabled flow still has its automatic-dispatch side effect bound
/// immediately (e.g. the schedule-trigger cron job registered), reusing the
/// same [`bind_trigger`] helper `flows_set_enabled` uses — but per Rule 1
/// that now only happens for a `manual`-triggered (or trigger-kind-less)
/// flow. Best-effort, same as `flows_set_enabled`: a binding failure is
/// logged, not fatal to create.
pub async fn flows_create(
    config: &Config,
    name: String,
    graph_json: Value,
    require_approval: bool,
) -> Result<Outcome<Flow>, String> {
    let graph = validate_and_migrate_graph(graph_json)?;
    ensure_config_aware_engine_compatible(config, &graph)?;

    // Rule 1: automatic triggers create DISABLED — the user must arm them
    // explicitly.
    let enabled = !trigger_is_automatic(&graph);

    // Rule 2: any outbound side-effect node forces require_approval, no
    // matter what the caller asked for.
    let (effective_require_approval, side_effect_forced) =
        enforce_side_effect_approval(&graph, require_approval);
    if side_effect_forced {
        tracing::info!(
            target: "flows",
            %name,
            "[flows] flows_create: forcing require_approval=true — graph contains outbound \
             side-effect node(s) (tool_call / http_request / code)"
        );
    }

    tracing::debug!(
        target: "flows",
        %name,
        node_count = graph.nodes.len(),
        enabled,
        require_approval = effective_require_approval,
        "[flows] flows_create: persisting new flow"
    );
    let flow = store::create_flow(config, name, graph, effective_require_approval, enabled)
        .map_err(|e| e.to_string())?;

    if flow.enabled {
        tracing::debug!(target: "flows", flow_id = %flow.id, "[flows] flows_create: flow is enabled — binding automatic-dispatch trigger");
        bind_trigger(config, &flow);
    }

    let mut logs = vec!["flow created".to_string()];
    if !enabled {
        let trigger_label = flow
            .graph
            .trigger()
            .and_then(|t| t.config.get("trigger_kind"))
            .and_then(Value::as_str)
            .unwrap_or("automatic");
        logs.push(format!(
            "Flow created DISABLED because it has an automatic trigger ({trigger_label}). \
             Enable it explicitly (flows_set_enabled) when you are ready for it to fire."
        ));
    }
    if side_effect_forced {
        logs.push(
            "require_approval forced to true because the graph contains outbound side-effect \
             nodes (tool_call / http_request / code)."
                .to_string(),
        );
    }

    publish_flow_changed(&flow.id, "created", "system");
    Ok(Outcome::new(flow, logs))
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

/// Loads a saved flow's portable [`WorkflowGraph`] by id, for the
/// `sub_workflow`-by-`workflow_id` resolver capability
/// (`tinyflows::caps::WorkflowResolver`, implemented in
/// `crates/openhuman-core/src/flows/tinyflows/caps.rs`).
///
/// Returns `Ok(None)` when no flow with that id exists (the resolver turns that
/// into a capability error naming the missing id), and `Err` only on a store
/// failure. Kept sync (the underlying [`store::get_flow`] is sync) so the
/// resolver can call it directly from its async method without a runtime hop.
pub fn load_flow_graph(config: &Config, id: &str) -> Result<Option<WorkflowGraph>, String> {
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
pub(crate) fn load_engine_compatible_flow_graph(
    config: &Config,
    id: &str,
) -> Result<Option<WorkflowGraph>, String> {
    let graph = load_flow_graph(config, id)?;
    if let Some(graph) = graph.as_ref() {
        ensure_config_aware_engine_compatible(config, graph)
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
/// Unbinds the flow's automatic-dispatch trigger (e.g. the schedule-trigger
/// cron job) *before* removing the flow definition. `flow_runs` cascades on
/// delete via a same-database `FOREIGN KEY ... ON DELETE CASCADE`, but a
/// bound cron job lives in the entirely separate `cron.db` — it does NOT
/// cascade — so skipping this would orphan the cron job, leaving it pointing
/// at a now-nonexistent `flow_id` forever. Best-effort: a lookup failure
/// (flow already gone, store error) is logged and does not block the delete
/// itself — `store::remove_flow` below still errors clearly if `id` doesn't
/// exist.
pub async fn flows_delete(config: &Config, id: &str) -> Result<Outcome<Value>, String> {
    match store::get_flow(config, id) {
        Ok(Some(flow)) => unbind_trigger(config, &flow),
        Ok(None) => {}
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %id, error = %e, "[flows] flows_delete: failed to load flow before unbind — proceeding with delete anyway");
        }
    }

    store::remove_flow(config, id).map_err(|e| e.to_string())?;
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
