
/// Fold Composio connected accounts + named HTTP credentials into the flat,
/// secret-free [`FlowConnection`] picker list. Only ACTIVE Composio connections
/// are surfaced — a pending/expired OAuth account cannot execute a tool, so it
/// would be a dead pick. Pure (no I/O) so the aggregation shape is
/// unit-testable without a live backend; `identities` is loaded once by the
/// caller and matched in here.
///
/// Each Composio connection is also matched against `identities` (keyed by
/// `(toolkit, connection_id)`, both normalized the same way
/// `enrich_connections_with_identity` in `composio::ops::connections` does)
/// to attach `platform_user_id` — the connected account's own member id
/// (e.g. Slack `U123ABC`). This is what lets the workflow builder wire a
/// self-targeted action ("DM me") to the user's own account instead of
/// guessing a public channel.
fn build_flow_connections(
    composio: Vec<crate::openhuman::integrations::composio::ComposioConnection>,
    http: Vec<crate::openhuman::security::credentials::HttpCredentialSummary>,
    identities: &[crate::openhuman::integrations::composio::providers::ConnectedIdentity],
) -> Vec<FlowConnection> {
    use tinymemory_api::composio::normalize_connection_identifier;

    let identity_lookup: std::collections::HashMap<(String, String), &_> = identities
        .iter()
        .map(|id| {
            (
                (
                    normalize_connection_identifier(&id.source),
                    normalize_connection_identifier(&id.identifier),
                ),
                id,
            )
        })
        .collect();

    let mut out = Vec::with_capacity(composio.len() + http.len());
    for conn in composio {
        if !conn.is_active() {
            tracing::debug!(
                toolkit = %conn.toolkit,
                connection_id = %conn.id,
                status = %conn.status,
                "[flows] flows_list_connections: skipping non-active composio connection"
            );
            continue;
        }
        let toolkit = conn.normalized_toolkit();
        let lookup_key = (
            normalize_connection_identifier(&toolkit),
            normalize_connection_identifier(&conn.id),
        );
        let platform_user_id = identity_lookup
            .get(&lookup_key)
            .and_then(|identity| identity.user_id.clone());
        tracing::debug!(
            toolkit = %toolkit,
            connection_id = %conn.id,
            has_platform_user_id = platform_user_id.is_some(),
            "[flows] flows_list_connections: resolved platform_user_id for composio connection"
        );
        out.push(FlowConnection {
            // Exactly the shape `tinyflows::caps::composio_connection_id` parses.
            connection_ref: format!("composio:{}:{}", toolkit, conn.id),
            kind: "composio".to_string(),
            display: composio_connection_display(&toolkit, &conn),
            toolkit: Some(toolkit),
            scheme: None,
            platform_user_id,
        });
    }
    for cred in http {
        out.push(FlowConnection {
            // Exactly the shape `tinyflows::caps::http_cred_name` parses.
            connection_ref: format!("http_cred:{}", cred.name),
            kind: "http".to_string(),
            display: http_credential_display(&cred),
            toolkit: None,
            scheme: Some(cred.scheme),
            platform_user_id: None,
        });
    }
    out
}

/// Human-readable picker label for a Composio connected account, e.g.
/// `"Gmail · user@example.com"`. Prefers email, then workspace/team, then
/// handle; falls back to the title-cased toolkit alone when no identity is
/// cached. The identity fields are display metadata (already surfaced by
/// `composio_list_connections`), never secret material.
fn composio_connection_display(
    toolkit: &str,
    conn: &crate::openhuman::integrations::composio::ComposioConnection,
) -> String {
    let title = title_case_toolkit(toolkit);
    let identity = conn
        .account_email
        .as_deref()
        .or(conn.workspace.as_deref())
        .or(conn.username.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    match identity {
        Some(id) => format!("{title} · {id}"),
        None => title,
    }
}

/// Human-readable picker label for a named HTTP credential, e.g.
/// `"stripe (bearer)"`. Only the (non-secret) name + scheme — never the value.
fn http_credential_display(
    cred: &crate::openhuman::security::credentials::HttpCredentialSummary,
) -> String {
    format!("{} ({})", cred.name, cred.scheme)
}

/// Title-case a toolkit slug for display: `"gmail"` → `"Gmail"`,
/// `"google_calendar"` → `"Google Calendar"`. Best-effort cosmetic only.
fn title_case_toolkit(toolkit: &str) -> String {
    let trimmed = toolkit.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    trimmed
        .split(['_', '-', ' '])
        .filter(|w| !w.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Publishes a [`DomainEvent::FlowChanged`](crate::core::events::DomainEvent::FlowChanged)
/// so an open Workflows list/canvas refetches (bridged to a `flow:changed`
/// socket event) — the observability half of audit F6. Best-effort broadcast;
/// `actor` is a coarse hint (`"system"` for RPC-driven changes today).
fn publish_flow_changed(flow_id: &str, kind: &str, actor: &str) {
    tracing::debug!(target: "flows", %flow_id, kind, actor, "[flows] publishing FlowChanged");
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::FlowChanged {
        flow_id: flow_id.to_string(),
        kind: kind.to_string(),
        actor: actor.to_string(),
    });
    // Re-advertise the workflow set to the medulla backend. This is the single
    // funnel every store mutation passes through (create / duplicate / update /
    // delete / enable), and the backend replaces a socket's whole entry on each
    // registration — so re-sending here is what keeps a remote orchestrator from
    // reasoning about a set that no longer exists. A no-op (one debug log, no
    // task spawned) when no bridge is installed, which is every build that is
    // not talking to a backend, and every test.
    crate::openhuman::platform::socket::medulla::workflows::emit_register_workflows();
}

/// Maps a store-level [`FlowUpdateError`](store::FlowUpdateError) to the RPC
/// error string. A concurrency conflict is encoded as a JSON object the UI can
/// parse (`{ code: "version_conflict", message, current }`) so it can offer a
/// reload/diff instead of silently clobbering; other variants are plain text.
fn map_flow_update_error(e: store::FlowUpdateError) -> String {
    match e {
        store::FlowUpdateError::NotFound => "flow not found".to_string(),
        store::FlowUpdateError::Conflict(current) => serde_json::to_string(&json!({
            "code": "version_conflict",
            "message": "This flow changed since you loaded it. Reload to see the latest \
                        version, then reapply your change.",
            "current": *current,
        }))
        .unwrap_or_else(|_| "version_conflict".to_string()),
        store::FlowUpdateError::Store(err) => err.to_string(),
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
) -> Result<RpcOutcome<Flow>, String> {
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

/// Update a flow while atomically disarming any automatic-trigger graph.
///
/// Remote authoring surfaces use this variant so revising a schedule,
/// app-event, or webhook flow never preserves a prior local opt-in to run the
/// old graph. The same guarded store write persists the graph and
/// `enabled=false`, so no trigger can observe the revised graph armed between
/// two writes.
pub(crate) async fn flows_update_disarming_automatic(
    config: &Config,
    id: &str,
    name: Option<String>,
    graph_json: Option<Value>,
    require_approval: Option<bool>,
    expected_version: Option<String>,
) -> Result<RpcOutcome<Flow>, String> {
    flows_update_inner(
        config,
        id,
        name,
        graph_json,
        require_approval,
        expected_version,
        true,
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
) -> Result<RpcOutcome<Flow>, String> {
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
    Ok(RpcOutcome::new(updated, logs))
}

/// Lists a flow's revision history (prior graph snapshots), newest first,
/// capped at `limit` (audit F6). The safety rail that makes rollback possible.
pub fn flows_get_history(
    config: &Config,
    id: &str,
    limit: usize,
) -> Result<RpcOutcome<Vec<crate::openhuman::flows::FlowRevision>>, String> {
    let revisions = store::list_revisions(config, id, limit).map_err(|e| e.to_string())?;
    let count = revisions.len();
    Ok(RpcOutcome::single_log(
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
) -> Result<RpcOutcome<Flow>, String> {
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
pub async fn flows_delete(config: &Config, id: &str) -> Result<RpcOutcome<Value>, String> {
    flows_delete_impl(config, id, None).await
}

/// Backs [`flows_delete`]. `memory_override`, when `Some`, is the guarded
/// driver used for the namespace-clear step below in place of the one
/// `memory::ops::guard::active_memory_guard` resolves — the same seam, and now
/// the same type, as `bus::FlowRunDigestSubscriber`'s `with_memory`.
///
/// # Why an override at all
///
/// `active_memory_guard` resolves the ambient `CoreContext`'s workspace, and a
/// pre-boot unit test has no context — it falls back to the single shared test
/// workspace that every `memory::ops` fixture writes into, not to the
/// `tempdir` this call's `config` names. A test asserting that *this* clear
/// step ran therefore has to be handed the binding over its own workspace, or
/// it is asserting against a store it never wrote to.
///
/// # What changed (#5560)
///
/// This used to take a `tinymemory_core::store::MemoryClientRef` — a direct
/// handle on the in-process engine, and the only reason this file named the
/// engine crate at all. It is an `Arc<MemoryGuard>` now, so the injected path
/// and the resolved path are the same type running the same policy steps; the
/// override can no longer be a second, unguarded door into memory. Production
/// still passes `None`.
async fn flows_delete_impl(
    config: &Config,
    id: &str,
    memory_override: Option<Arc<crate::openhuman::memory::guard::MemoryGuard>>,
) -> Result<RpcOutcome<Value>, String> {
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
    if let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() {
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

    // Best-effort: clear this flow's private memory namespace along with its
    // row — a deleted flow must not leave stray `flow_memory_remember`
    // entries or run digests behind. Never fails the delete itself: the flow
    // row is already gone by this point regardless of what happens here.
    let memory_namespace = flow_namespace(id);
    let guard = match memory_override {
        Some(guard) => Ok(guard),
        None => crate::openhuman::memory::ops::guard::active_memory_guard().await,
    };
    let clear_result = match guard {
        Ok(guard) => {
            tracing::debug!(target: "flows", flow_id = %id, namespace = %memory_namespace, driver = %guard.driver_id(), "[flows] flows_delete: clearing flow memory namespace through the bound driver");
            match guard.as_documents() {
                Some(documents) => documents
                    .clear_namespace(&memory_namespace)
                    .await
                    .map_err(|error| error.to_string()),
                // Name the driver: "does not support" with no subject reads as
                // a host bug, and the actual fact is which driver is bound.
                None => Err(format!(
                    "the bound memory driver '{}' does not serve the documents family",
                    guard.driver_id()
                )),
            }
        }
        Err(error) => Err(error),
    };
    if let Err(error) = clear_result {
        tracing::warn!(target: "flows", flow_id = %id, namespace = %memory_namespace, %error, "[flows] flows_delete: failed to clear flow memory namespace");
    }

    publish_flow_changed(id, "deleted", "system");
    Ok(RpcOutcome::new(
        json!({ "id": id, "removed": true }),
        vec![format!("flow removed: {id}")],
    ))
}

/// Enables or disables a flow. Enable/disable now (B2) binds/tears down the
/// flow's automatic trigger:
/// - `schedule` — registers/removes the backing `cron` job
///   (`cron::add_flow_schedule_job` / `cron::remove_job`) so
///   `flows::bus::FlowTriggerSubscriber` gets a `FlowScheduleTick` on the
///   configured cadence.
/// - `app_event` — no enable-time side effect needed: the subscriber matches
///   every `ComposioTriggerReceived` against `store::list_enabled_flows` at
///   dispatch time, so the `enabled` flag alone gates it.
/// - `webhook` — **not implemented** in B2 (best-effort deviation, see
///   `bind_trigger`'s webhook arm below and
///   `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §1); logged,
///   not silently skipped.
/// - `manual` / anything else — no binding needed; `flows_run` always works.
///
/// `flows_run` still runs a disabled flow on demand (mirrors
/// `cron::rpc::cron_run`'s "Run Now always works" behavior) — `enabled` only
/// gates *automatic* trigger-driven dispatch.
pub async fn flows_set_enabled(
    config: &Config,
    id: &str,
    enabled: bool,
) -> Result<RpcOutcome<Flow>, String> {
    let flow = store::set_enabled(config, id, enabled).map_err(|e| e.to_string())?;

    if enabled {
        bind_trigger(config, &flow);
    } else {
        unbind_trigger(config, &flow);
    }

    let mut logs = vec![format!("flow {id} enabled={enabled}")];
    // When enabling, loudly surface any unfired-trigger-kind warning in the
    // result (a structured `warning:`-prefixed log), not just a silent tracing
    // line — so an enable of a flow that will never fire itself (webhook,
    // chat_message, form, …) is impossible to miss at the call site.
    if enabled {
        for warning in graph_trigger_warnings(&flow.graph) {
            tracing::warn!(
                target: "flows",
                flow_id = %id,
                warning = %warning,
                "[flows] flows_set_enabled: enabling a flow whose trigger kind does not fire yet"
            );
            logs.push(format!("warning: {warning}"));
        }
    }

    publish_flow_changed(id, "enabled_changed", "system");
    Ok(RpcOutcome::new(flow, logs))
}

/// Registers the automatic-dispatch side effect for `flow`'s trigger kind, if
/// any. Best-effort: a binding failure is logged and does not fail the
/// `flows_set_enabled` call — the flow is still saved as enabled, it just
/// won't fire automatically until the underlying issue (invalid schedule,
/// cron store error, …) is fixed.
fn bind_trigger(config: &Config, flow: &Flow) {
    match bus::extract_trigger_kind(flow) {
        Some(TriggerKind::Schedule) => bind_schedule_trigger(config, flow),
        Some(TriggerKind::Webhook) => log_webhook_trigger_deferred(flow, true),
        _ => {
            // `app_event` needs no enable-time binding (matched at dispatch
            // time against `list_enabled_flows`); `manual`/`form`/others have
            // no automatic-dispatch concept at all.
        }
    }
}

/// Tears down the automatic-dispatch side effect for `flow`'s trigger kind,
/// mirroring [`bind_trigger`]. Best-effort, same rationale.
fn unbind_trigger(config: &Config, flow: &Flow) {
    match bus::extract_trigger_kind(flow) {
        Some(TriggerKind::Schedule) => unbind_schedule_trigger(config, &flow.id),
        Some(TriggerKind::Webhook) => log_webhook_trigger_deferred(flow, false),
        _ => {}
    }
}
