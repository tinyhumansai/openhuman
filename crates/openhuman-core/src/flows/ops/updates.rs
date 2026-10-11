use super::*;

/// Maps a store-level [`FlowUpdateError`](tinyflows_sqlite::flows::FlowUpdateError) to the RPC
/// error string. A concurrency conflict is encoded as a JSON object the UI can
/// parse (`{ code: "version_conflict", message, current }`) so it can offer a
/// reload/diff instead of silently clobbering; other variants are plain text.
fn map_flow_update_error(e: tinyflows_sqlite::flows::FlowUpdateError) -> String {
    match e {
        tinyflows_sqlite::flows::FlowUpdateError::NotFound => "flow not found".to_string(),
        tinyflows_sqlite::flows::FlowUpdateError::Conflict(current) => {
            serde_json::to_string(&json!({
                "code": "version_conflict",
                "message": "This flow changed since you loaded it. Reload to see the latest \
                            version, then reapply your change.",
                "current": *current,
            }))
            .unwrap_or_else(|_| "version_conflict".to_string())
        }
        tinyflows_sqlite::flows::FlowUpdateError::Store(err) => err.to_string(),
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
/// `webhook`, see [`trigger_is_automatic`]) as `enabled`, but that guard only
/// runs once, at creation. Without an equivalent here, a flow created
/// `enabled: true` with a manual/no-op trigger could later have an
/// automatic-trigger graph saved onto it — via the `save_workflow` agent
/// tool, the canvas Save button, a proposal apply, or any other
/// `flows_update` caller — and go LIVE immediately with no user review
/// (confirmed live: a flow started firing on an unreviewed 8am schedule).
/// So: when the *new* graph's trigger is automatic and the *previous*
/// graph's trigger was NOT automatic (a manual/none → automatic
/// transition), this forces the persisted `enabled` back to `false` in the
/// same store write — the user must explicitly re-arm via
/// `flows_set_enabled` after reviewing the new trigger. An automatic →
/// automatic re-edit (e.g. tweaking a cron expression) is left alone — the
/// user already opted in once, and re-disarming on every edit would just be
/// friction.
///
/// The override is applied **unconditionally** on a manual/none → automatic
/// transition — it does *not* gate on whether the flow *looked* enabled in
/// the `existing` read above. That read is a snapshot taken before
/// `store::update_flow_graph`'s own guarded UPDATE re-reads the row; a
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
    flows_update_inner(
        config,
        id,
        name,
        graph_json,
        require_approval,
        expected_version,
        false,
    )
    .await
}

async fn flows_update_inner(
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
            let graph = validate_and_migrate_graph(raw)?;
            ensure_config_aware_engine_compatible(config, &graph)?;
            graph
        }
        None => {
            tinyflows::validate::validate(&existing.graph).map_err(|e| e.to_string())?;
            existing.graph.clone()
        }
    };
    // B29 Rule 1 analogue: disarm every manual/none → automatic trigger
    // transition, unconditionally. `now_auto` is safe to compute here (it
    // only depends on `graph`, THIS call's own incoming graph — never
    // stale). The "was it automatic before" half of the transition,
    // however, is NOT decided here: R-m2 found that gating on the
    // ops-level `existing.graph` read let a concurrent write race this
    // call and slip an automatic-trigger graph through with `enabled: true`
    // — `existing` can be arbitrarily stale by the time
    // `store::update_flow_graph` actually performs its guarded write. That
    // decision now lives inside `update_flow_graph`, computed against the
    // row it just re-read there (see its doc comment).
    let now_auto = trigger_is_automatic(&graph);
    let forced_automatic_disarm = disarm_automatic && now_auto;
    tracing::debug!(
        target: "flows",
        flow_id = %id,
        now_auto,
        currently_enabled = existing.enabled,
        forced_automatic_disarm,
        "[flows] flows_update: auto-trigger disarm decision inputs (transition itself decided \
         store-side against a fresh read, see update_flow_graph)"
    );

    // Rule 2 analogue (compound-bypass closure): re-apply the same outbound
    // side-effect check `flows_create` applies on save — via the shared
    // [`enforce_side_effect_approval`] helper — so an update that *adds* a
    // tool_call/http_request/code node to a previously read-only graph can
    // never persist `require_approval: false` just because the update path
    // trusted the caller's toggle unconditionally.
    let (effective_require_approval, side_effect_forced) =
        enforce_side_effect_approval(&graph, new_require_approval);
    if side_effect_forced {
        tracing::info!(
            target: "flows",
            flow_id = %id,
            "[flows] flows_update: forcing require_approval=true — graph contains outbound \
             side-effect node(s) (tool_call / http_request / code)"
        );
    }

    tracing::debug!(
        target: "flows",
        flow_id = %id,
        has_expected = expected_version.is_some(),
        require_approval = effective_require_approval,
        side_effect_forced,
        "[flows] flows_update: persisting changes"
    );
    // The auto-disarm decision (both the unconditional manual→automatic
    // transition and `disarm_automatic`'s forced-remote-authoring variant)
    // is made INSIDE `update_flow_graph`, against the row it re-reads right
    // before its guarded UPDATE — see R-m2 above and that function's doc
    // comment. `enabled_override: None` here means "no explicit force from
    // this caller"; the disarm, if any, still applies on top of that.
    let updated = store::update_flow_graph(
        config,
        id,
        new_name,
        graph,
        effective_require_approval,
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
    let should_disarm = now_auto && existing.enabled && !updated.enabled;
    if should_disarm {
        tracing::info!(
            target: "flows",
            flow_id = %id,
            "[flows] flows_update: auto-disabled automatic-trigger graph pending explicit re-arm"
        );
    }

    if graph_changed && updated.enabled {
        let trigger_unchanged = bus::extract_trigger_kind(&existing)
            == bus::extract_trigger_kind(&updated)
            && bus::extract_trigger_config(&existing) == bus::extract_trigger_config(&updated);
        if !trigger_unchanged {
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
    if side_effect_forced {
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
) -> Result<Outcome<Vec<tinyflows_catalog::FlowRevision>>, String> {
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
    let rev = store::revision_by_id(config, id, revision_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("revision '{revision_id}' not found for flow '{id}'"))?;

    tracing::debug!(target: "flows", flow_id = %id, %revision_id, "[flows] flows_rollback: restoring prior revision");
    flows_update(
        config,
        id,
        Some(rev.name),
        Some(rev.graph),
        Some(rev.require_approval),
        expected_version,
    )
    .await
}
