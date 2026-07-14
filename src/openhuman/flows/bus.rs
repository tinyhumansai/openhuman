//! Event bus handlers for the `flows::` domain (issue B2 — see
//! `my_docs/ohxtf/b2-triggers-trust/01-triggers-and-trust.md` §1).
//!
//! [`FlowTriggerSubscriber`] is the trigger → run bridge: it listens for the
//! normalized events a saved flow's trigger node can bind to
//! (`DomainEvent::FlowScheduleTick`, `ComposioTriggerReceived`,
//! `WebhookIncomingRequest`), matches them against enabled flows, and spawns
//! `flows::ops::flows_run` for each match. Matching helpers
//! ([`extract_trigger_kind`], [`extract_trigger_config`]) are also reused by
//! `flows::ops::flows_set_enabled` to bind/unbind a flow's automatic
//! dispatch on enable/disable.

use crate::core::event_bus::{DomainEvent, EventHandler};
use crate::openhuman::config::Config;
use crate::openhuman::flows::store;
use crate::openhuman::flows::Flow;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tinyflows::model::TriggerKind;

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

/// True when `flow` is an enabled `app_event` flow bound to the given
/// Composio `toolkit`/`trigger_slug` (case-insensitive — Composio slugs are
/// conventionally upper-case but authoring surfaces may not normalize them).
fn matches_app_event(flow: &Flow, toolkit: &str, trigger_slug: &str) -> bool {
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
    fn try_acquire_dispatch(&self, flow_id: &str) -> Option<InFlightGuard> {
        let mut in_flight = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        if !in_flight.insert(flow_id.to_string()) {
            return None;
        }
        Some(InFlightGuard {
            set: self.in_flight.clone(),
            flow_id: flow_id.to_string(),
        })
    }

    /// `DomainEvent::FlowScheduleTick` — a `flow`-type cron job fired. Loads
    /// the one named flow, checks it is still enabled with a `schedule`
    /// trigger (it may have been disabled/edited since the job was
    /// registered), and dispatches it with an empty trigger payload.
    async fn handle_schedule_tick(&self, flow_id: &str) {
        let flow = match store::get_flow(&self.config, flow_id) {
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
        self.spawn_run(
            flow_id.to_string(),
            Value::Null,
            crate::openhuman::flows::FlowRunTrigger::Schedule,
        );
    }

    /// `DomainEvent::ComposioTriggerReceived` — scans every enabled flow for
    /// an `app_event` trigger bound to this `toolkit`/`trigger_slug` and
    /// dispatches each match with the event payload as the run input
    /// (seeded into `run.trigger`, per the node-catalog contract).
    async fn handle_app_event(&self, toolkit: &str, trigger_slug: &str, payload: &Value) {
        let flows = match store::list_enabled_flows(&self.config) {
            Ok(flows) => flows,
            Err(e) => {
                tracing::warn!(target: "flows", %toolkit, %trigger_slug, error = %e, "[flows] failed to list enabled flows for app_event dispatch");
                return;
            }
        };

        let mut matched = 0usize;
        for flow in flows {
            if matches_app_event(&flow, toolkit, trigger_slug) {
                matched += 1;
                self.spawn_run(
                    flow.id.clone(),
                    payload.clone(),
                    crate::openhuman::flows::FlowRunTrigger::AppEvent,
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
        flow_id: String,
        input: Value,
        trigger: crate::openhuman::flows::FlowRunTrigger,
    ) {
        let Some(guard) = self.try_acquire_dispatch(&flow_id) else {
            tracing::debug!(target: "flows", %flow_id, "[flows] trigger: flow already running — skipping this tick");
            return;
        };

        let config = self.config.clone();
        tokio::spawn(async move {
            // Held for the lifetime of the run; released on drop (including
            // on panic) by `InFlightGuard`.
            let _guard = guard;
            tracing::info!(target: "flows", %flow_id, "[flows] trigger fired — starting run");
            match crate::openhuman::flows::ops::flows_run(&config, &flow_id, input, trigger).await {
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
struct InFlightGuard {
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
impl EventHandler for FlowTriggerSubscriber {
    fn name(&self) -> &str {
        "flows::trigger"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["cron", "composio", "webhook", "system"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::FlowScheduleTick { flow_id } => self.handle_schedule_tick(flow_id).await,
            DomainEvent::ComposioTriggerReceived {
                toolkit,
                trigger,
                payload,
                ..
            } => self.handle_app_event(toolkit, trigger, payload).await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::flows::Flow;
    use serde_json::json;
    use tinyflows::model::{Node, NodeKind, WorkflowGraph};

    fn test_config(tmp: &tempfile::TempDir) -> Arc<Config> {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            action_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        Arc::new(config)
    }

    fn trigger_node(config: Value) -> Node {
        Node {
            id: "t".to_string(),
            kind: NodeKind::Trigger,
            type_version: 1,
            name: "Trigger".to_string(),
            config,
            ports: Vec::new(),
            position: None,
        }
    }

    fn flow_with_trigger_config(id: &str, enabled: bool, trigger_config: Value) -> Flow {
        Flow {
            id: id.to_string(),
            name: id.to_string(),
            enabled,
            graph: WorkflowGraph {
                nodes: vec![trigger_node(trigger_config)],
                ..Default::default()
            },
            created_at: "2026-01-01T00:00:00Z".to_string(),
            updated_at: "2026-01-01T00:00:00Z".to_string(),
            last_run_at: None,
            last_status: None,
            require_approval: false,
        }
    }

    #[test]
    fn name_and_domains_are_stable() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = FlowTriggerSubscriber::new(test_config(&tmp));
        assert_eq!(sub.name(), "flows::trigger");
        assert_eq!(
            sub.domains(),
            Some(&["cron", "composio", "webhook", "system"][..])
        );
    }

    #[tokio::test]
    async fn handle_does_not_panic_on_arbitrary_events() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = FlowTriggerSubscriber::new(test_config(&tmp));
        sub.handle(&DomainEvent::CronJobTriggered {
            job_id: "j1".into(),
            job_name: "test".into(),
            job_type: "shell".into(),
        })
        .await;
        sub.handle(&DomainEvent::FlowScheduleTick {
            flow_id: "missing-flow".into(),
        })
        .await;
    }

    #[test]
    fn extract_trigger_kind_reads_schedule() {
        let flow = flow_with_trigger_config(
            "f1",
            true,
            json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
        );
        assert!(matches!(
            extract_trigger_kind(&flow),
            Some(TriggerKind::Schedule)
        ));
    }

    #[test]
    fn extract_trigger_kind_none_for_missing_discriminator() {
        let flow = flow_with_trigger_config("f1", true, json!({}));
        assert!(extract_trigger_kind(&flow).is_none());
    }

    #[test]
    fn extract_trigger_kind_none_for_invalid_discriminator() {
        let flow = flow_with_trigger_config("f1", true, json!({ "trigger_kind": "not_a_kind" }));
        assert!(extract_trigger_kind(&flow).is_none());
    }

    #[test]
    fn matches_app_event_requires_toolkit_and_slug_match() {
        let flow = flow_with_trigger_config(
            "f1",
            true,
            json!({ "trigger_kind": "app_event", "toolkit": "gmail", "trigger_slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
        );
        assert!(matches_app_event(&flow, "gmail", "GMAIL_NEW_GMAIL_MESSAGE"));
        // Case-insensitive.
        assert!(matches_app_event(&flow, "Gmail", "gmail_new_gmail_message"));
        // Wrong toolkit or slug does not match.
        assert!(!matches_app_event(
            &flow,
            "slack",
            "GMAIL_NEW_GMAIL_MESSAGE"
        ));
        assert!(!matches_app_event(&flow, "gmail", "SLACK_NEW_MESSAGE"));
    }

    #[test]
    fn matches_app_event_false_for_non_app_event_trigger() {
        let flow = flow_with_trigger_config(
            "f1",
            true,
            json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
        );
        assert!(!matches_app_event(
            &flow,
            "gmail",
            "GMAIL_NEW_GMAIL_MESSAGE"
        ));
    }

    #[tokio::test]
    async fn handle_app_event_ignores_disabled_flows() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = test_config(&tmp);
        let flow = flow_with_trigger_config(
            "disabled-flow",
            false,
            json!({ "trigger_kind": "app_event", "toolkit": "gmail", "trigger_slug": "GMAIL_NEW_GMAIL_MESSAGE" }),
        );
        crate::openhuman::flows::store::upsert_flow(&config, &flow).unwrap();

        // `list_enabled_flows` must not surface the disabled flow at all —
        // proves the subscriber's dispatch source already excludes it,
        // rather than asserting on a spawned background task's side effect.
        let enabled = crate::openhuman::flows::store::list_enabled_flows(&config).unwrap();
        assert!(enabled.is_empty());
    }

    #[tokio::test]
    async fn handle_schedule_tick_ignores_disabled_flow() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = test_config(&tmp);
        let flow = flow_with_trigger_config(
            "sched-flow",
            false,
            json!({ "trigger_kind": "schedule", "schedule": "0 9 * * *" }),
        );
        crate::openhuman::flows::store::upsert_flow(&config, &flow).unwrap();

        let sub = FlowTriggerSubscriber::new(config.clone());
        // Must not panic and must not spawn a run for a disabled flow — we
        // can't directly observe "no run happened" without a full flows_run
        // fixture, but this exercises the early-return path without error.
        sub.handle(&DomainEvent::FlowScheduleTick {
            flow_id: "sched-flow".into(),
        })
        .await;
    }

    // ── in-flight dedupe (CodeRabbit finding B) ─────────────────────

    #[test]
    fn try_acquire_dispatch_skips_a_flow_already_in_flight() {
        let tmp = tempfile::TempDir::new().unwrap();
        let sub = FlowTriggerSubscriber::new(test_config(&tmp));

        let guard = sub
            .try_acquire_dispatch("f1")
            .expect("first claim for f1 should succeed");
        assert!(
            sub.try_acquire_dispatch("f1").is_none(),
            "a second claim for the same flow while the first is held must be skipped"
        );

        // A different flow is unaffected.
        assert!(sub.try_acquire_dispatch("f2").is_some());

        drop(guard);
        assert!(
            sub.try_acquire_dispatch("f1").is_some(),
            "dropping the guard must release the claim so f1 can run again"
        );
    }

    #[test]
    fn default_constructs_the_same_as_new() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = test_config(&tmp);
        let a = FlowTriggerSubscriber::new(config.clone());
        let b = FlowTriggerSubscriber::new(config);
        assert_eq!(a.name(), b.name());
    }
}
