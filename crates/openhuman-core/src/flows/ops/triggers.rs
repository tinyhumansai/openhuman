use super::*;

/// Enables or disables a flow. Enable/disable now (B2) binds/tears down the
/// flow's automatic trigger:
/// - `schedule` — registers/removes the backing `cron` job
///   (`cron::add_flow_schedule_job` / `cron::remove_job`) so
///   `flows::bus::FlowTriggerSubscriber` gets a schedule trigger from the
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
) -> Result<Outcome<Flow>, String> {
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
        match serde_json::to_value(&flow.graph) {
            Ok(graph) => match super::validation::validate_over_bus(
                &crate::core::runtime::DispatchCtx::ambient(),
                config,
                graph,
            )
            .await
            {
                Ok(analysis) => {
                    for warning in analysis.warnings {
                        tracing::warn!(
                            target: "flows",
                            flow_id = %id,
                            warning = %warning,
                            "[flows] flows_set_enabled: enabling a flow whose trigger kind does not fire yet"
                        );
                        logs.push(format!("warning: {warning}"));
                    }
                }
                Err(error) => tracing::warn!(
                    target: "flows",
                    flow_id = %id,
                    %error,
                    "[flows] flows_set_enabled: TinyFlows warning analysis failed"
                ),
            },
            Err(error) => tracing::warn!(
                target: "flows",
                flow_id = %id,
                %error,
                "[flows] flows_set_enabled: graph serialization failed during warning analysis"
            ),
        }
    }

    publish_flow_changed(id, "enabled_changed", "system");
    Ok(Outcome::new(flow, logs))
}

/// Registers the automatic-dispatch side effect for `flow`'s trigger kind, if
/// any. Best-effort: a binding failure is logged and does not fail the
/// `flows_set_enabled` call — the flow is still saved as enabled, it just
/// won't fire automatically until the underlying issue (invalid schedule,
/// cron store error, …) is fixed.
pub(super) fn bind_trigger(config: &Config, flow: &Flow) {
    let details = match bus::describe_trigger(config, flow) {
        Ok(details) => details,
        Err(error) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, %error, "[flows] failed to describe trigger while binding");
            return;
        }
    };
    match details.kind.as_deref() {
        Some("schedule") => match details.schedule {
            Some(schedule) => bind_schedule_trigger(config, flow, schedule),
            None => {
                tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger description omitted its schedule")
            }
        },
        Some("webhook") => log_webhook_trigger_deferred(flow, true),
        _ => {
            // `app_event` is matched at dispatch time; other kinds have no
            // enable-time automatic-dispatch binding.
        }
    }
}

/// Tears down the automatic-dispatch side effect for `flow`'s trigger kind,
/// mirroring [`bind_trigger`]. Best-effort, same rationale.
pub(super) fn unbind_trigger(config: &Config, flow: &Flow) {
    let details = match bus::describe_trigger(config, flow) {
        Ok(details) => details,
        Err(error) => {
            tracing::warn!(target: "flows", flow_id = %flow.id, %error, "[flows] failed to describe trigger while unbinding");
            return;
        }
    };
    match details.kind.as_deref() {
        Some("schedule") => unbind_schedule_trigger(config, &flow.id),
        Some("webhook") => log_webhook_trigger_deferred(flow, false),
        _ => {}
    }
}

/// Registers (or refreshes) the `cron` job backing a `schedule`-trigger
/// flow. Idempotent — re-uses an existing binding via
/// `cron::find_flow_schedule_job` rather than creating a duplicate, so this
/// is safe to call both from `flows_set_enabled` and from boot
/// reconciliation ([`reconcile_schedule_triggers_on_boot`]).
pub(super) fn bind_schedule_trigger(
    config: &Config,
    flow: &Flow,
    schedule: tinyflows_bus::schedule::types::Schedule,
) {
    match crate::cron::find_flow_schedule_job(config, &flow.id) {
        Ok(Some(existing)) => {
            let patch = tinyflows_bus::schedule::types::CronJobPatch {
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
    // `local`, then every agent that keeps its flows in its own storage scope
    // (`crate::storage::agents`); each scope's flows get their cron jobs in
    // that scope, where the scheduler visits them.
    let mut errors = Vec::new();
    for (agent, result) in crate::storage::agents::for_each_scope(
        &crate::core::runtime::DispatchCtx::host_default(),
        "flows schedule reconcile",
        |_| reconcile_schedule_triggers_in_scope(config),
    )
    .await
    {
        if let Err(error) = result {
            errors.push(format!("{}: {error}", agent.as_deref().unwrap_or("local")));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// [`reconcile_schedule_triggers_on_boot`] for the current storage scope.
async fn reconcile_schedule_triggers_in_scope(config: &Config) -> Result<(), String> {
    let (flows, skipped) = store::list_enabled_flows(config).map_err(|e| e.to_string())?;
    if skipped > 0 {
        // R-M4: a corrupt/unmigratable row must not abort boot reconciliation
        // for every other enabled flow — skipped rows are logged loudly
        // (never their content) so the gap is diagnosable.
        tracing::warn!(target: "flows", skipped, "[flows] reconcile_schedule_triggers_on_boot: skipped corrupt/unmigratable flow rows");
    }
    let mut reconciled = 0usize;
    for flow in &flows {
        match bus::describe_trigger(config, flow) {
            Ok(details) if details.kind.as_deref() == Some("schedule") => {
                if let Some(schedule) = details.schedule {
                    bind_schedule_trigger(config, flow, schedule);
                    reconciled += 1;
                } else {
                    tracing::warn!(target: "flows", flow_id = %flow.id, "[flows] schedule trigger description omitted its schedule during boot reconcile");
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(target: "flows", flow_id = %flow.id, %error, "[flows] failed to describe trigger during boot reconcile")
            }
        }
    }
    tracing::debug!(target: "flows", scanned = flows.len(), reconciled, skipped, "[flows] boot reconciliation of schedule-trigger cron jobs complete");
    Ok(())
}
