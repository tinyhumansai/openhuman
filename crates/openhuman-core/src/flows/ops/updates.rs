use super::*;

/// Maps the Catalog bus [`FlowUpdateError`](tinyflows_bus::catalog::FlowUpdateError) to the RPC
/// error string. A concurrency conflict is encoded as a JSON object the UI can
/// parse (`{ code: "version_conflict", message, current }`) so it can offer a
/// reload/diff instead of silently clobbering; other variants are plain text.
fn map_flow_update_error(e: tinyflows_bus::catalog::FlowUpdateError) -> String {
    match e {
        tinyflows_bus::catalog::FlowUpdateError::NotFound { .. } => "flow not found".to_string(),
        tinyflows_bus::catalog::FlowUpdateError::VersionConflict { message, current } => {
            serde_json::to_string(&json!({
                "code": "version_conflict",
                "message": message,
                "current": current,
            }))
            .unwrap_or_else(|_| "version_conflict".to_string())
        }
        tinyflows_bus::catalog::FlowUpdateError::Store { message } => message,
    }
}

/// Updates a flow's name, graph, and/or `require_approval` toggle.
/// Re-validates the graph (whether newly supplied or the existing one)
/// before persisting, same as `flows_create`.
///
/// When the caller supplies a new `graph_json` and the flow is (still)
/// enabled, re-binds the automatic-dispatch trigger if the trigger
/// kind/config actually changed (e.g. a new schedule cron expression) —
/// otherwise the stale binding from the old graph would keep firing on the
/// old cadence, or a newly-added schedule would never get bound at all.
/// Skipped entirely for a name/`require_approval`-only update (no
/// `graph_json` supplied), since the trigger definitely didn't change.
///
/// **B29 Rule 1 analogue for saves** (save/enable safety — same issue
/// `flows_create` guards at creation time, see its doc): `flows_create`
/// refuses to persist an automatic-trigger graph (`schedule` / `app_event` /
/// `webhook`) as `enabled`, but that guard only
/// runs once, at creation. Without an equivalent here, a flow created
/// `enabled: true` with a manual/no-op trigger could later have an
/// automatic-trigger graph saved onto it — via the `save_workflow` agent
/// tool, the canvas Save button, a proposal apply, or any other
/// `flows_update` caller — and go LIVE immediately with no user review
/// (confirmed live: a flow started firing on an unreviewed 8am schedule).
/// So: when the *new* graph's trigger is automatic and the *previous*
/// graph's trigger was NOT automatic (a manual/none → automatic
/// transition), this forces the persisted `enabled` back to `false` in the
/// the same Catalog write — the user must explicitly re-arm via
/// `flows_set_enabled` after reviewing the new trigger. An automatic →
/// automatic re-edit (e.g. tweaking a cron expression) is left alone — the
/// user already opted in once, and re-disarming on every edit would just be
/// friction.
///
/// The override is applied **unconditionally** on a manual/none → automatic
/// transition — it does *not* gate on whether the flow *looked* enabled in
/// the `existing` read above. That read is a snapshot taken before
/// Catalog.PutWorkflow's guarded UPDATE re-reads the row; a
/// concurrent `flows_set_enabled(id, true)` landing in the gap would leave
/// this snapshot stale while the row is actually enabled by the time the
/// guarded UPDATE runs — and since `set_enabled` bumps `updated_at` too,
/// such a race wouldn't even trip the optimistic-concurrency conflict, it
/// would just silently persist the automatic graph as enabled (the exact
/// bug this rule exists to close). Gating on the stale `existing.enabled`
/// re-opens that race; forcing the override on every transition, enabled-or-
/// not, is exactly as safe as Rule 1's at-create version — a transition on
/// an already-disabled flow is just a no-op write of `enabled=false` over
/// `enabled=false`.
pub async fn flows_update(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
) -> Result<Outcome<Flow>, String> {
    flows_update_with_context(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        id,
        name,
        graph_json,
        require_approval,
        expected_version,
    )
    .await
}

pub(crate) async fn flows_update_with_context(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
) -> Result<Outcome<Flow>, String> {
    ctx.enter(async move {
        flows_update_inner(
            ctx,
            config,
            id,
            name,
            graph_json,
            require_approval,
            expected_version,
            false,
        )
        .await
    })
    .await
}

async fn flows_update_inner(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
    disarm_automatic: bool,
) -> Result<Outcome<Flow>, String> {
    let existing = store::get_flow(config, id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("flow '{id}' not found"))?;

    let new_name = name.unwrap_or_else(|| existing.name.clone());
    let new_require_approval = require_approval.unwrap_or(existing.require_approval);
    let graph_changed = graph_json.is_some();
    let graph = match graph_json {
        Some(raw) => {
            let graph = validate_and_migrate_graph(ctx, config, raw).await?;
            ensure_config_aware_engine_compatible(ctx, config, &graph).await?;
            graph
        }
        // Metadata-only updates preserve access to legacy graphs written by
        // older versions, even when the current engine no longer accepts them.
        None => {
            validate_and_migrate_graph(ctx, config, existing.graph.clone()).await?;
            existing.graph.clone()
        }
    };
    let forced_automatic_disarm = disarm_automatic;
    // Catalog.PutWorkflow owns the automatic-trigger transition rule. It
    // evaluates the new graph and fresh persisted row in the module before
    // writing, and reports the result through `born_disabled`.
    tracing::debug!(
        target: "flows",
        flow_id = %id,
        currently_enabled = existing.enabled,
        force_disarm_automatic = disarm_automatic,
        "[flows] flows_update: delegating automatic-trigger transition policy to Catalog.PutWorkflow"
    );

    tracing::debug!(
        target: "flows",
        flow_id = %id,
        has_expected = expected_version.is_some(),
        require_approval = new_require_approval,
        "[flows] flows_update: persisting changes"
    );
    // The module computes automatic-trigger disarm against its freshly read
    // row; the explicit request flag is reserved for remote-authoring flows.
    let updated = store::update_flow_graph(
        config,
        id,
        new_name,
        graph,
        new_require_approval,
        None,
        disarm_automatic,
        expected_version.as_deref(),
    )
    .map_err(map_flow_update_error)?;

    // Best-effort, POST-write: did the flow actually transition from
    // enabled to disabled as part of this update? Derived from the real
    // before/after state (`existing.enabled` vs `updated.enabled`) rather
    // than re-predicting the decision — the decision itself already
    // happened store-side against a fresh read, so this is purely for the
    // info log / result message wording below and can't desync from what
    // was actually persisted.
    let tinyflows_bus::catalog::PutWorkflowResult {
        workflow: updated,
        born_disabled: should_disarm,
        approval_forced,
    } = updated;
    if should_disarm {
        tracing::info!(
            target: "flows",
            flow_id = %id,
            "[flows] flows_update: auto-disabled automatic-trigger graph pending explicit re-arm"
        );
    }

    if graph_changed && updated.enabled {
        let rebind = match (
            bus::describe_trigger(config, &existing),
            bus::describe_trigger(config, &updated),
        ) {
            (Ok(previous), Ok(current)) => previous != current,
            (previous, current) => {
                if let Err(error) = previous {
                    tracing::warn!(target: "flows", flow_id = %id, %error, "[flows] could not describe previous trigger during update");
                }
                if let Err(error) = current {
                    tracing::warn!(target: "flows", flow_id = %id, %error, "[flows] could not describe updated trigger during update");
                }
                // Trigger details are best-effort. Refresh when either
                // description fails, so an old schedule is not kept live
                // after a malformed schedule edit.
                true
            }
        };
        if rebind {
            tracing::debug!(target: "flows", flow_id = %id, "[flows] flows_update: trigger changed on an enabled flow — rebinding automatic-dispatch trigger");
            unbind_trigger(config, &existing);
            bind_trigger(config, &updated);
        }
    }

    publish_flow_changed(id, "updated", "system");
    let mut logs = vec![format!("flow updated: {id}")];
    if should_disarm {
        let reason = if forced_automatic_disarm {
            "Flow was auto-disabled because this authoring surface revised an automatic trigger \
             (schedule / app_event / webhook). Enable it explicitly (flows_set_enabled) once \
             you've reviewed the revision."
        } else {
            "Flow was auto-disabled because its trigger changed from manual to automatic \
             (schedule / app_event / webhook). Enable it explicitly (flows_set_enabled) once \
             you've reviewed the new trigger."
        };
        logs.push(reason.to_string());
    }
    if approval_forced {
        logs.push(
            "require_approval forced to true because the graph contains outbound side-effect \
             nodes (tool_call / http_request / code)."
                .to_string(),
        );
    }
    Ok(Outcome::new(updated, logs))
}

/// Lists a flow's revision history (prior graph snapshots), newest first,
/// capped at `limit` (audit F6). The safety rail that makes rollback possible.
pub fn flows_get_history(
    config: &Config,
    id: &str,
    limit: usize,
) -> Result<Outcome<Vec<tinyflows_bus::catalog::WorkflowRevision>>, String> {
    let revisions = store::list_revisions(config, id, limit).map_err(|e| e.to_string())?;
    let count = revisions.len();
    Ok(Outcome::single_log(
        revisions,
        format!("flow history: {id} ({count} revisions)"),
    ))
}

/// Rolls a flow back to a prior revision by restoring that revision's graph
/// through the normal update path — which itself snapshots the current graph as
/// a new revision, so a rollback is itself undoable. Honours optimistic
/// concurrency via `expected_version`.
pub async fn flows_rollback(
    config: &Config,
    id: &str,
    revision_id: &str,
    expected_version: Option<String>,
) -> Result<Outcome<Flow>, String> {
    flows_rollback_with_context(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        id,
        revision_id,
        expected_version,
    )
    .await
}

pub(crate) async fn flows_rollback_with_context(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    id: &str,
    revision_id: &str,
    expected_version: Option<String>,
) -> Result<Outcome<Flow>, String> {
    ctx.enter(async move {
        let rev = store::revision_by_id(config, id, revision_id)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("revision '{revision_id}' not found for flow '{id}'"))?;

        tracing::debug!(target: "flows", flow_id = %id, %revision_id, "[flows] flows_rollback: restoring prior revision");
        flows_update_with_context(
            ctx,
            config,
            id,
            Some(rev.name),
            Some(rev.graph),
            Some(rev.require_approval),
            expected_version,
        )
        .await
    })
    .await
}
