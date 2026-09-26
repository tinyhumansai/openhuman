use super::*;

/// Stable snake_case label for a [`TriggerKind`], matching its serde wire
/// discriminator — used in loud author-facing warnings (not derived via serde
/// so the exact human string is unmistakable at the call site).
fn trigger_kind_label(kind: &TriggerKind) -> &'static str {
    match kind {
        TriggerKind::Manual => "manual",
        TriggerKind::Schedule => "schedule",
        TriggerKind::Webhook => "webhook",
        TriggerKind::AppEvent => "app_event",
        TriggerKind::Form => "form",
        TriggerKind::ExecuteByWorkflow => "execute_by_workflow",
        TriggerKind::ChatMessage => "chat_message",
        TriggerKind::Evaluation => "evaluation",
        TriggerKind::System => "system",
    }
}

/// Whether a flow's trigger kind currently produces *automatic* runs in this
/// host. Only three kinds fire today:
/// - `manual` — runnable on demand via `flows_run` (no automatic dispatch, but
///   that's the whole contract of a manual trigger — never a surprise).
/// - `schedule` — a `cron` job drives `FlowScheduleTick` (see
///   [`bind_schedule_trigger`]).
/// - `app_event` — matched against `ComposioTriggerReceived` at dispatch time
///   (see `flows::bus::FlowTriggerSubscriber`).
///
/// Everything else (`webhook`, `chat_message`, `form`, `execute_by_workflow`,
/// `evaluation`, `system`) is *accepted and saved* but has no wired dispatch
/// path yet — enabling such a flow silently produces a flow that never runs
/// itself. [`graph_trigger_warnings`] turns that silence into a loud warning.
fn trigger_kind_fires(kind: &TriggerKind) -> bool {
    matches!(
        kind,
        TriggerKind::Manual | TriggerKind::Schedule | TriggerKind::AppEvent
    )
}

/// Produces host-side, **non-fatal** validation warnings for a graph — today
/// exactly one: "this trigger kind does not fire automatically yet". Returns
/// an empty vec when the trigger fires (`manual`/`schedule`/`app_event`), when
/// the graph has no single resolvable trigger node, or when the trigger has no
/// `trigger_kind` discriminator (a legacy/manual-only graph authored before
/// B2 simply never self-fires — not a warnable surprise, matching
/// `bus::extract_trigger_kind`'s "no automatic binding" treatment).
///
/// This lives host-side (NOT in `tinyflows::validate`, which is host-agnostic
/// and only does structural checks) because "which trigger kinds this host has
/// wired" is an OpenHuman fact, not a property of the portable graph.
pub(crate) fn graph_trigger_warnings(graph: &WorkflowGraph) -> Vec<String> {
    let Some(trigger) = graph.trigger() else {
        return Vec::new();
    };
    let Some(kind_value) = trigger.config.get("trigger_kind") else {
        return Vec::new();
    };
    let kind: TriggerKind = match serde_json::from_value(kind_value.clone()) {
        Ok(k) => k,
        Err(_) => return Vec::new(),
    };
    if trigger_kind_fires(&kind) {
        return Vec::new();
    }
    let label = trigger_kind_label(&kind);
    vec![format!(
        "Trigger kind '{label}' does not fire automatically yet — this flow will be saved and \
         can be enabled, but nothing will run it on its own until that trigger is wired up. Run \
         it manually with flows_run, or switch to a `schedule` or `app_event` trigger."
    )]
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
pub(super) fn bind_trigger(config: &Config, flow: &Flow) {
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
pub(super) fn unbind_trigger(config: &Config, flow: &Flow) {
    match bus::extract_trigger_kind(flow) {
        Some(TriggerKind::Schedule) => unbind_schedule_trigger(config, &flow.id),
        Some(TriggerKind::Webhook) => log_webhook_trigger_deferred(flow, false),
        _ => {}
    }
}

/// Registers (or refreshes) the `cron` job backing a `schedule`-trigger
/// flow. Idempotent — re-uses an existing binding via
/// `cron::find_flow_schedule_job` rather than creating a duplicate, so this
/// is safe to call both from `flows_set_enabled` and from boot
/// reconciliation ([`reconcile_schedule_triggers_on_boot`]).
pub(super) fn bind_schedule_trigger(config: &Config, flow: &Flow) {
    let Some(trigger_config) = bus::extract_trigger_config(flow) else {
        tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger: flow has no single trigger node — cannot bind cron job");
        return;
    };
    let Some(schedule_raw) = trigger_config.get("schedule").cloned() else {
        tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger config is missing `schedule` — cannot bind cron job");
        return;
    };
    let schedule: crate::cron::Schedule = match serde_json::from_value(schedule_raw) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, error = %e, "[flows] invalid schedule trigger config — cannot bind cron job");
            return;
        }
    };

    match crate::cron::find_flow_schedule_job(config, &flow.id) {
        Ok(Some(existing)) => {
            let patch = crate::cron::CronJobPatch {
                enabled: Some(true),
                schedule: Some(schedule),
                ..Default::default()
            };
            if let Err(e) = crate::cron::update_job(config, &existing.id, patch) {
                tracing::warn!(target: "flows", flow_id = %flow.id, cron_job_id = %existing.id, error = %e, "[flows] failed to refresh existing schedule-trigger cron job");
            } else {
                tracing::debug!(target: "flows", flow_id = %flow.id, cron_job_id = %existing.id, "[flows] refreshed existing schedule-trigger cron job");
            }
        }
        Ok(None) => match crate::cron::add_flow_schedule_job(config, &flow.id, schedule) {
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
    match crate::cron::find_flow_schedule_job(config, flow_id) {
        Ok(Some(job)) => {
            if let Err(e) = crate::cron::remove_job(config, &job.id) {
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
/// (`openhuman.webhooks_create_tunnel`, served by `openhuman-tinyhumans`, a
/// network call to the signed-in backend
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
