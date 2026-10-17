//! Trigger → run bridge: [`FlowTriggerSubscriber`] listens for the
//! normalized events a saved flow's trigger node can bind to and spawns
//! `flows::ops::flows_run` for each match, plus the trigger-config readers
//! `flows::ops` reuses to bind/unbind automatic dispatch.

use crate::config::Config;
use crate::core::events::DomainEvent;
use crate::flows::store;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tinybus::EventHandler;
use tinyflows::model::TriggerKind;
use tinyflows_catalog::Flow;

/// Reads `trigger_kind` from a flow's trigger node config, deserializing into
/// `tinyflows::model::TriggerKind`. Returns `None` when the flow doesn't have
/// exactly one trigger node ([`tinyflows::model::WorkflowGraph::trigger`]) or
/// the `trigger_kind` discriminator is missing/invalid — callers treat that
/// as "no automatic binding", not an error (a `manual`-only or legacy graph
/// authored before B2 simply never fires itself).
pub(crate) fn extract_trigger_kind(flow: &Flow) -> Option<TriggerKind> {
    let trigger = flow.graph.trigger()?;
    serde_json::from_value(trigger.config.get("trigger_kind")?.clone()).ok()
}

/// Returns the trigger node's full config value, for callers that need
/// kind-specific fields (`schedule` for `schedule`, `toolkit`/`trigger_slug`
/// for `app_event`, …).
pub(crate) fn extract_trigger_config(flow: &Flow) -> Option<&Value> {
    Some(&flow.graph.trigger()?.config)
}

/// Values an author pinned on the trigger node for *unattended* runs, read from
/// the trigger's `config.inputs` object.
///
/// A schedule tick or an inbound app event has no operator to prompt, so a flow
/// with declared inputs would otherwise be undispatchable. Pinning values in the
/// trigger config is how such a flow states, at author time, what an automatic
/// run should use. Values are passed through literally — this is configuration,
/// not an expression scope, and there is no run in flight to resolve one
/// against.
///
/// Returns an empty map when the trigger declares none, in which case a required
/// input with no default fails in `prepare_flow_run` before any run row exists,
/// and the reason is logged and visible in the run digest.
pub(super) fn pinned_trigger_inputs(flow: &Flow) -> serde_json::Map<String, Value> {
    extract_trigger_config(flow)
        .and_then(|cfg| cfg.get("inputs"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// True when `flow` is an enabled `app_event` flow bound to the given
/// Composio `toolkit`/`trigger_slug` (case-insensitive — Composio slugs are
/// conventionally upper-case but authoring surfaces may not normalize them).
pub(super) fn matches_app_event(flow: &Flow, toolkit: &str, trigger_slug: &str) -> bool {
    if !matches!(extract_trigger_kind(flow), Some(TriggerKind::AppEvent)) {
        return false;
    }
    let Some(cfg) = extract_trigger_config(flow) else {
        return false;
    };
    let cfg_toolkit = cfg.get("toolkit").and_then(Value::as_str).unwrap_or("");
    let cfg_slug = cfg
        .get("trigger_slug")
        .and_then(Value::as_str)
        .unwrap_or("");
    cfg_toolkit.eq_ignore_ascii_case(toolkit) && cfg_slug.eq_ignore_ascii_case(trigger_slug)
}

/// Listens for normalized trigger events and starts runs for matching
/// enabled flows. See the module doc for the full contract.
pub struct FlowTriggerSubscriber {
    config: Arc<Config>,
    /// Process-local dedupe of trigger-driven dispatch, keyed by `flow_id`
    /// (CodeRabbit finding B — overlapping runs for the same flow). A fast
    /// cadence or trigger burst can otherwise fire `spawn_run` for the same
    /// flow multiple times before the first run finishes, racing
    /// `last_run_at`/`last_status` and doing duplicate work. This is
    /// intentionally scoped to trigger-driven dispatch (this subscriber) —
    /// the interactive `flows_run` RPC is NOT deduped, since a user
    /// explicitly asking to run a flow again (e.g. while a scheduled run is
    /// still in flight) is fine.
    in_flight: Arc<Mutex<HashSet<String>>>,
}

impl FlowTriggerSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            in_flight: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Attempts to claim `flow_id` for a trigger-driven dispatch. Returns
    /// `None` when a dispatch for the same flow is already in flight — the
    /// caller should skip this tick. Returns `Some(guard)` on success; the
    /// guard releases the claim on `Drop` (including on panic/early return),
    /// so a run can never permanently wedge the flow out of future ticks.
    pub(super) fn try_acquire_dispatch(&self, flow_id: &str) -> Option<InFlightGuard> {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if !in_flight.insert(flow_id.to_string()) {
            return None;
        }
        Some(InFlightGuard {
            set: self.in_flight.clone(),
            flow_id: flow_id.to_string(),
        })
    }

    /// The configuration to handle an event under (`super::owner::config_for_scope`).
    pub(super) fn config_for_scope(&self, ctx: &crate::core::runtime::DispatchCtx) -> Arc<Config> {
        super::owner::config_for_scope(ctx, &self.config)
    }

    /// A `flow`-type cron job fired (`cron::scheduler`'s trigger sink calls
    /// this directly). Runs as the agent the flow belongs to
    /// (`super::owner`), then loads the one named flow, checks it is still
    /// enabled with a `schedule` trigger (it may have been disabled/edited
    /// since the job was registered), and dispatches it with an empty trigger
    /// payload.
    pub(crate) async fn dispatch_schedule_tick(
        &self,
        host: &crate::core::runtime::DispatchCtx,
        flow_id: &str,
    ) {
        super::owner::as_owner(host, &self.config, flow_id, |ctx| {
            self.handle_schedule_tick(ctx, flow_id)
        })
        .await
    }

    async fn handle_schedule_tick(&self, ctx: crate::core::runtime::DispatchCtx, flow_id: &str) {
        let config = self.config_for_scope(&ctx);
        let flow = match store::get_flow(&config, flow_id) {
            Ok(Some(flow)) => flow,
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, "[flows] schedule tick for unknown/removed flow — ignoring");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, error = %e, "[flows] failed to load flow for schedule tick");
                return;
            }
        };
        if !flow.enabled {
            tracing::debug!(target: "flows", %flow_id, "[flows] schedule tick for disabled flow — ignoring");
            return;
        }
        if !matches!(extract_trigger_kind(&flow), Some(TriggerKind::Schedule)) {
            tracing::debug!(target: "flows", %flow_id, "[flows] schedule tick for flow whose trigger is no longer `schedule` — ignoring");
            return;
        }
        let inputs = pinned_trigger_inputs(&flow);
        self.spawn_run(
            &ctx,
            flow_id.to_string(),
            Value::Null,
            inputs,
            tinyflows_catalog::FlowRunTrigger::Schedule,
        );
    }

    /// `DomainEvent::ComposioTriggerReceived` — scans every enabled flow for
    /// an `app_event` trigger bound to this `toolkit`/`trigger_slug` and
    /// dispatches each match with the event payload as the run input
    /// (seeded into `run.trigger`, per the node-catalog contract).
    async fn handle_app_event(
        &self,
        ctx: crate::core::runtime::DispatchCtx,
        toolkit: &str,
        trigger_slug: &str,
        payload: &Value,
    ) {
        let config = self.config_for_scope(&ctx);
        let (flows, skipped) = match store::list_enabled_flows(&config) {
            Ok(result) => result,
            Err(e) => {
                tracing::warn!(target: "flows", %toolkit, %trigger_slug, error = %e, "[flows] failed to list enabled flows for app_event dispatch");
                return;
            }
        };
        if skipped > 0 {
            // R-M4: one corrupt/unmigratable flow row must not blackhole
            // app_event dispatch for every other enabled flow.
            tracing::warn!(target: "flows", %toolkit, %trigger_slug, skipped, "[flows] handle_app_event: skipped corrupt/unmigratable flow rows while matching trigger");
        }

        let mut matched = 0usize;
        for flow in flows {
            if matches_app_event(&flow, toolkit, trigger_slug) {
                matched += 1;
                let inputs = pinned_trigger_inputs(&flow);
                self.spawn_run(
                    &ctx,
                    flow.id.clone(),
                    payload.clone(),
                    inputs,
                    tinyflows_catalog::FlowRunTrigger::AppEvent,
                );
            }
        }
        tracing::debug!(target: "flows", %toolkit, %trigger_slug, matched, "[flows] app_event trigger matching complete");
    }

    /// Spawns a background `flows::ops::flows_run` for `flow_id`. Fire-and-
    /// forget from the bus's perspective — `flows_run` itself records the
    /// outcome onto the flow's summary fields and a `flow_runs` history row,
    /// and surfaces a `CoreNotification` when the run pauses for approval.
    ///
    /// Skips the dispatch (see [`try_acquire_dispatch`]) if a trigger-driven
    /// run for this `flow_id` is already in flight, so a fast schedule or a
    /// burst of matching `app_event`s cannot run the same flow concurrently.
    fn spawn_run(
        &self,
        ctx: &crate::core::runtime::DispatchCtx,
        flow_id: String,
        input: Value,
        inputs: serde_json::Map<String, Value>,
        trigger: tinyflows_catalog::FlowRunTrigger,
    ) {
        let Some(guard) = self.try_acquire_dispatch(&flow_id) else {
            tracing::debug!(target: "flows", %flow_id, "[flows] trigger: flow already running — skipping this tick");
            return;
        };

        let config = self.config_for_scope(ctx);
        let run_ctx = ctx.clone();
        // Scoped: the run keeps the owner's context the handler entered.
        ctx.spawn(async move {
            // Held for the lifetime of the run; released on drop (including
            // on panic) by `InFlightGuard`.
            let _guard = guard;
            tracing::info!(target: "flows", %flow_id, "[flows] trigger fired — starting run");
            match crate::flows::ops::flows_run(&run_ctx, &config, &flow_id, input, inputs, trigger).await {
                Ok(_) => {
                    tracing::info!(target: "flows", %flow_id, "[flows] trigger-driven run finished")
                }
                Err(e) => {
                    tracing::warn!(target: "flows", %flow_id, error = %e, "[flows] trigger-driven run failed")
                }
            }
        });
    }
}

/// Drop guard releasing a [`FlowTriggerSubscriber::try_acquire_dispatch`]
/// claim. Removing the `flow_id` on `Drop` (rather than only on the happy
/// path) means a panicking or erroring `flows_run` still frees the flow up
/// for its next trigger tick.
pub(super) struct InFlightGuard {
    set: Arc<Mutex<HashSet<String>>>,
    flow_id: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        // Recover from a poisoned lock (mirrors `try_acquire_dispatch`) so the
        // flow_id is always removed — otherwise a poison would wedge this flow
        // out of every future trigger dispatch, defeating the guard's purpose.
        let mut set = self.set.lock().unwrap_or_else(|e| e.into_inner());
        set.remove(&self.flow_id);
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for FlowTriggerSubscriber {
    fn name(&self) -> &str {
        "flows::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron", "composio", "webhook", "system"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // Trigger events arrive over the host's socket or its own scheduler:
        // the host's own tenant, and in SaaS none.
        let host = crate::core::runtime::DispatchCtx::host_default();
        match event {
            // Every scope's flows may listen for this trigger: match them in
            // each, so each run starts as its flow's owner.
            DomainEvent::ComposioTriggerReceived {
                toolkit,
                trigger,
                payload,
                ..
            } => {
                crate::storage::agents::for_each_live_scope(&host, "flows app_event", |ctx| {
                    self.handle_app_event(ctx, toolkit, trigger, payload)
                })
                .await;
            }
            DomainEvent::WebhookIncomingRequest { .. } => {
                // Best-effort deviation (documented, not silently skipped —
                // see `flows::ops::log_webhook_trigger_deferred` for the
                // enable/disable-side note): a `webhook`-trigger flow needs a
                // backend-provisioned tunnel + a UI surface for the resulting
                // URL, neither of which exists yet. Never log the request's
                // `raw_data` here — it is untrusted, possibly-sensitive
                // inbound payload.
                tracing::debug!(
                    target: "flows",
                    "[flows] observed WebhookIncomingRequest — webhook-trigger dispatch is not \
                     implemented in B2 (pending backend tunnel provisioning + B3 UI); no flow \
                     dispatched"
                );
            }
            other => {
                // Anything else on our filtered domains (plain shell/agent
                // `CronJobTriggered`, other Composio lifecycle events,
                // system lifecycle, …) is not a flow trigger — ignore. Log
                // only the variant name, never the event's Debug form: some
                // sibling variants on these domains carry payloads we must
                // not put in logs (e.g. `ComposioTriggerReceived::payload`).
                tracing::trace!(target: "flows", variant = other.variant_name(), "[flows] ignoring unrelated event");
            }
        }
    }
}
