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

use crate::core::events::DomainEvent;
use crate::openhuman::config::Config;
use crate::openhuman::flows::store;
use crate::openhuman::flows::{flow_namespace, Flow, FlowRun};
use crate::openhuman::memory::{Memory, MemoryCategory, MemoryTaint};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use tinybus::EventHandler;
use tinyflows::model::{NodeKind, TriggerKind};
use tinyflows::nodes::control_flow::dedup as dedup_node;

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
fn pinned_trigger_inputs(flow: &Flow) -> serde_json::Map<String, Value> {
    extract_trigger_config(flow)
        .and_then(|cfg| cfg.get("inputs"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
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
        let inputs = pinned_trigger_inputs(&flow);
        self.spawn_run(
            flow_id.to_string(),
            Value::Null,
            inputs,
            crate::openhuman::flows::FlowRunTrigger::Schedule,
        );
    }

    /// `DomainEvent::ComposioTriggerReceived` — scans every enabled flow for
    /// an `app_event` trigger bound to this `toolkit`/`trigger_slug` and
    /// dispatches each match with the event payload as the run input
    /// (seeded into `run.trigger`, per the node-catalog contract).
    async fn handle_app_event(&self, toolkit: &str, trigger_slug: &str, payload: &Value) {
        let (flows, skipped) = match store::list_enabled_flows(&self.config) {
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
                    flow.id.clone(),
                    payload.clone(),
                    inputs,
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
        inputs: serde_json::Map<String, Value>,
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
            match crate::openhuman::flows::ops::flows_run(&config, &flow_id, input, inputs, trigger)
                .await
            {
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
impl EventHandler<DomainEvent> for FlowTriggerSubscriber {
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

/// Bounds a post-run memory digest to a compact, LLM-cheap size — a single
/// run's summary must never dominate a later `flow_memory_recall`.
const DIGEST_MAX_CHARS: usize = 1000;

/// Cap on how many `run_digest:*` entries [`FlowRunDigestSubscriber`] keeps
/// per flow's memory namespace before pruning the oldest.
const DIGEST_RETENTION_CAP: usize = 50;

/// Listens for `DomainEvent::FlowRunFinished` and, on a successful terminal
/// status, writes a compact digest of the run into the flow's own private
/// memory namespace ([`flow_namespace`]) — e.g. so a later run of the same
/// scheduled digest flow can `flow_memory_recall` what it already sent
/// without re-deriving that from the target service.
///
/// Success-only: `"failed"` / `"cancelled"` / `"interrupted"` / any other
/// terminal status is ignored, since a digest of a run that didn't actually
/// complete its work would misleadingly look like a record of real output.
///
/// Best-effort throughout: every failure here is logged via `tracing::warn!`
/// and swallowed, never propagated — by the time this subscriber observes
/// `FlowRunFinished`, the run has already settled its own `flow_runs` row, so
/// a memory-layer hiccup must never retroactively affect run status.
pub struct FlowRunDigestSubscriber {
    config: Arc<Config>,
    /// Test-only memory override. In production this is `None` and the digest
    /// resolves the process-global memory client via [`active_memory_client`].
    /// The process-global client is a one-shot `OnceLock`, so a unit test
    /// cannot reliably rebind it to its own tempdir (an earlier test in the
    /// same binary may already have initialised the singleton — see
    /// `memory::global`'s own test notes). Injecting a directly-constructed
    /// [`Memory`] here lets the digest tests write and read back through the
    /// SAME instance deterministically, exactly as `flows::memory_tools`'
    /// tests do with `UnifiedMemory::new`.
    memory_override: Option<Arc<dyn Memory>>,
}

impl FlowRunDigestSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            memory_override: None,
        }
    }

    /// Test constructor: run the digest against an explicitly-provided memory
    /// instance instead of the process-global client. See [`Self::memory_override`].
    #[cfg(test)]
    fn with_memory(config: Arc<Config>, memory: Arc<dyn Memory>) -> Self {
        Self {
            config,
            memory_override: Some(memory),
        }
    }

    /// Resolves the memory handle the digest writes to: the injected test
    /// override when present, else the process-global client
    /// ([`active_memory_client`]). Returns `None` (best-effort skip) when the
    /// global client is unavailable.
    async fn resolve_memory(&self) -> Option<Arc<dyn Memory>> {
        if let Some(memory) = &self.memory_override {
            return Some(memory.clone());
        }
        match crate::openhuman::memory::ops::helpers::active_memory_client().await {
            Ok(client) => Some(client.memory_handle()),
            Err(e) => {
                tracing::warn!(target: "flows", error = %e, "[flows] digest: memory client unavailable — skipping");
                None
            }
        }
    }

    async fn handle_finished(&self, flow_id: &str, run_id: &str, status: &str) {
        if status != "completed" && status != "completed_with_warnings" {
            tracing::trace!(target: "flows", %flow_id, %run_id, %status, "[flows] digest: ignoring non-success terminal status");
            return;
        }

        let flow_name = match store::get_flow(&self.config, flow_id) {
            Ok(Some(flow)) => flow.name,
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, %run_id, "[flows] digest: flow no longer exists — skipping");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, error = %e, "[flows] digest: failed to load flow — skipping");
                return;
            }
        };

        let run = match store::get_flow_run(&self.config, run_id) {
            Ok(Some(run)) => run,
            Ok(None) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, "[flows] digest: run row not found — skipping");
                return;
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, %run_id, error = %e, "[flows] digest: failed to load run — skipping");
                return;
            }
        };

        let digest = render_run_digest(&flow_name, &run);

        let Some(memory) = self.resolve_memory().await else {
            return;
        };
        let namespace = flow_namespace(flow_id);
        let digest_key = format!("run_digest:{run_id}");

        if let Err(e) = memory
            .store_with_taint(
                &namespace,
                &digest_key,
                &digest,
                MemoryCategory::Core,
                None,
                MemoryTaint::ExternalSync,
            )
            .await
        {
            tracing::warn!(target: "flows", %flow_id, %run_id, %namespace, error = %e, "[flows] digest: failed to write run digest");
            return;
        }

        self.enforce_retention_cap(&memory, &namespace).await;
    }

    /// Best-effort prune: keeps at most [`DIGEST_RETENTION_CAP`] `run_digest:*`
    /// entries per flow namespace, evicting the oldest (by `timestamp`) first.
    async fn enforce_retention_cap(&self, memory: &Arc<dyn Memory>, namespace: &str) {
        let entries = match memory.list(Some(namespace), None, None).await {
            Ok(entries) => entries,
            Err(e) => {
                tracing::warn!(target: "flows", %namespace, error = %e, "[flows] digest: retention sweep failed to list namespace");
                return;
            }
        };
        let mut digests: Vec<_> = entries
            .into_iter()
            .filter(|entry| entry.key.starts_with("run_digest:"))
            .collect();
        if digests.len() <= DIGEST_RETENTION_CAP {
            return;
        }
        // Oldest first, so the excess taken below is the stalest entries.
        digests.sort_by(|a, b| a.timestamp.cmp(&b.timestamp));
        let excess = digests.len() - DIGEST_RETENTION_CAP;
        for entry in digests.into_iter().take(excess) {
            if let Err(e) = memory.forget(namespace, &entry.key).await {
                tracing::warn!(target: "flows", %namespace, key = %entry.key, error = %e, "[flows] digest: retention sweep failed to forget stale entry");
            }
        }
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for FlowRunDigestSubscriber {
    fn name(&self) -> &str {
        "flows::digest"
    }

    fn domains(&self) -> Option<&[&str]> {
        // `FlowRunFinished` — the only event this subscriber handles — is
        // itself tagged `"cron"` by `DomainEvent::domain()` (grouped there
        // with the other flow-run/schedule events), not `"flows"`. This is
        // matching that tag, not a typo.
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::FlowRunFinished {
            flow_id,
            run_id,
            status,
        } = event
        {
            self.handle_finished(flow_id, run_id, status).await;
        }
    }
}

/// Truncates `s` to at most `max` `char`s, appending `…` when truncated.
fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{truncated}…")
}

/// Composes a compact, bounded summary of a finished run: flow name,
/// finished-at, status, node count, and per-node status + truncated output.
/// Bounded to [`DIGEST_MAX_CHARS`] total.
fn render_run_digest(flow_name: &str, run: &FlowRun) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "Flow: {flow_name}");
    let _ = writeln!(out, "Status: {}", run.status);
    if let Some(finished_at) = &run.finished_at {
        let _ = writeln!(out, "Finished: {finished_at}");
    }
    let _ = writeln!(out, "Nodes: {}", run.steps.len());
    for step in &run.steps {
        if out.chars().count() >= DIGEST_MAX_CHARS {
            break;
        }
        let status = step.status.as_deref().unwrap_or("?");
        let output = truncate_chars(&step.output.to_string(), 120);
        let _ = writeln!(out, "- {} [{status}]: {output}", step.node_id);
    }
    truncate_chars(&out, DIGEST_MAX_CHARS)
}

/// Listens for `DomainEvent::FlowRunFinished` and settles every `dedup` node
/// in the finished flow's graph — the host half of the commit-on-success
/// exactly-once contract the tinyflows `dedup` node depends on (issue #5263
/// PR2; the filter half — `DedupNode` — is PR1, already in `vendor/tinyflows`;
/// see `tinyflows::nodes::control_flow::dedup`'s module docs for the full
/// two-sided contract this subscriber implements).
///
/// For every `dedup` node found in the flow's saved graph:
/// - **Success** (`"completed"` / `"completed_with_warnings"`): unions the
///   node's `tentative` key set into its `committed` set, then clears
///   `tentative`. `completed_with_warnings` counts as success — the run
///   reached a terminal, non-retried outcome, so the items it processed are
///   genuinely done even if some non-fatal step warned.
/// - **Anything else** (`"failed"` / `"cancelled"` / `"interrupted"`, or any
///   future/unrecognized status string): clears `tentative` only, leaving
///   `committed` untouched, so the released keys are exactly as unseen as
///   before this run and the flow's next run reprocesses them. An
///   unrecognized status is deliberately treated as failure, not success —
///   "retry an already-done item" is always safe, "silently mark an
///   uncertain outcome as done" is not.
///
/// `StateStore` exposes no prefix-scan, so the only way to know which
/// `dedup:<node_id>:*` keys exist for a flow is to derive `<node_id>` from
/// the flow's own saved graph — this subscriber loads `flow_id`'s graph on
/// every event rather than trying to infer node ids from the event itself.
///
/// Reuses the exact same per-flow `StateStore` namespace
/// (`"flow:<flow_id>"`, see `tinyflows::caps::build_capabilities` in
/// `src/openhuman/flows/tinyflows/caps.rs`) the engine's `FlowStateStore` hands the
/// `dedup` node during the run — that collision with the node's own keys is
/// the entire point.
///
/// Best-effort throughout: every failure here is logged via `tracing::warn!`
/// and swallowed, never propagated — by the time this subscriber observes
/// `FlowRunFinished`, the run has already settled its own `flow_runs` row, so
/// a state-store hiccup here must never retroactively affect run status. A
/// failed commit degrades to "retry next run" (an item is reprocessed, never
/// lost); a failed release degrades to "stays tentative", which the `dedup`
/// node treats as unseen anyway since it only ever consults `committed` —
/// neither failure mode risks silently dropping an item.
///
/// **Commit atomicity (issue #5265, CodeRabbit "Major" on the dedup engine
/// PR):** the per-node commit itself is a read-modify-write
/// (`load(committed) → union(tentative) → store(committed) → delete
/// (tentative)`), not a compare-and-swap. Two overlapping `FlowRunFinished`
/// events for the SAME `flow_id` (e.g. a scheduled run and a manual re-run
/// racing each other) could otherwise interleave their read-modify-writes
/// and have the second writer's `store(committed)` clobber the first
/// writer's union, silently losing that run's committed keys
/// (last-writer-wins). [`handle_finished`](Self::handle_finished) closes
/// that DURABLE half of the race by serializing all of a given flow's
/// dedup-node settlement through a per-`flow_id` lock (see
/// [`FLOW_COMMIT_LOCKS`]) — different flows never contend. This does NOT
/// fix the node-side half: the `dedup` node's own in-run `StateStore`
/// read-modify-write (a single run unioning its own newly-seen items into
/// `tentative`) is a separate, still-open limitation documented on
/// `tinyflows::nodes::control_flow::dedup`'s side; a full CAS-based
/// `StateStore` is deferred.
pub struct DedupCommitSubscriber {
    config: Arc<Config>,
    /// Test-only instrumentation — see [`CommitTestHooks`]. Always `None` in
    /// production (`DedupCommitSubscriber::new`).
    #[cfg(test)]
    test_hooks: Option<Arc<CommitTestHooks>>,
}

/// Process-global registry of per-flow commit locks (issue #5265). Keyed by
/// `flow_id` so unrelated flows never contend with each other; the shared
/// `tokio::sync::Mutex<()>` per key lets [`DedupCommitSubscriber::
/// handle_finished`] hold a guard across its whole (synchronous)
/// read-modify-write section for that flow. Mirrors the same
/// `LazyLock<Mutex<HashMap<K, Arc<tokio::sync::Mutex<()>>>>>` keyed-lock
/// idiom `update_memory_md`'s `WORKSPACE_WRITE_LOCKS` uses for an analogous
/// read-modify-write race (#4458) — grepped for an existing pattern before
/// adding this one; that's the closest match in the crate.
///
/// Deliberately unbounded, matching that precedent: flow ids are bounded in
/// practice (a user's saved flow set), so an evicting map would be
/// complexity this doesn't need yet.
static FLOW_COMMIT_LOCKS: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Returns (creating if needed) the shared async commit lock for `flow_id`.
fn flow_commit_lock(flow_id: &str) -> Arc<tokio::sync::Mutex<()>> {
    let mut map = FLOW_COMMIT_LOCKS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(
        map.entry(flow_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(()))),
    )
}

/// Test-only scheduling/witness hooks for proving [`FLOW_COMMIT_LOCKS`]'
/// mutual exclusion. Deliberately **instance-scoped** (owned by one
/// [`DedupCommitSubscriber`], via [`DedupCommitSubscriber::with_test_hooks`])
/// rather than a process-global static: cargo's test harness runs different
/// `#[tokio::test]` functions concurrently on separate OS threads, and a
/// global counter would have unrelated tests' ordinary (unarmed,
/// effectively-instant) commits interleave with — and pollute — a
/// concurrency test's high-water-mark measurement purely by scheduling
/// chance. Scoping the hooks to one test's own `Arc` means only tasks that
/// share that specific subscriber instance can ever touch its counters.
#[cfg(test)]
#[derive(Default)]
struct CommitTestHooks {
    delay_ms: std::sync::atomic::AtomicU64,
    concurrent: std::sync::atomic::AtomicUsize,
    max_concurrent: std::sync::atomic::AtomicUsize,
}

impl DedupCommitSubscriber {
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            #[cfg(test)]
            test_hooks: None,
        }
    }

    /// Test constructor: attaches [`CommitTestHooks`] so a test can arm a
    /// delay inside the commit critical section and observe how many
    /// `handle_finished` calls were concurrently inside it.
    #[cfg(test)]
    fn with_test_hooks(config: Arc<Config>, hooks: Arc<CommitTestHooks>) -> Self {
        Self {
            config,
            test_hooks: Some(hooks),
        }
    }

    /// No-op unless [`Self::with_test_hooks`] attached hooks — awaited right
    /// after `handle_finished` acquires the per-flow commit lock, while
    /// still holding it. This is what makes it possible to force two
    /// spawned tasks to genuinely interleave on a single-threaded test
    /// executor (there are no other `.await` points inside the
    /// commit/release critical section to give the executor a chance to
    /// poll a contending task) — a test can then prove the lock, not
    /// accidental scheduling luck, is what serializes two overlapping
    /// `FlowRunFinished` events for the same flow. Compiles to an empty
    /// async fn body (zero-cost) in non-test builds.
    async fn maybe_test_delay(&self) {
        #[cfg(test)]
        if let Some(hooks) = &self.test_hooks {
            use std::sync::atomic::Ordering;
            let now = hooks.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
            hooks.max_concurrent.fetch_max(now, Ordering::SeqCst);

            let ms = hooks.delay_ms.load(Ordering::SeqCst);
            if ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            }

            hooks.concurrent.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// The node ids of every `dedup` node the finishing run executed.
    ///
    /// Prefers the per-run snapshot [`snapshot_run_dedup_nodes`] wrote at
    /// run-start (issue #5268 item 2), so a flow edited *while the run was
    /// still in flight* is settled against the graph the run actually ran
    /// rather than whatever the flow has since been edited into. This closes
    /// the case where a `dedup` node the run wrote `tentative` keys under is
    /// deleted or renamed before `FlowRunFinished` fires: previously it was
    /// no longer found here, so those keys were neither committed nor
    /// released and the items silently retried on the flow's next run.
    ///
    /// Falls back to the flow's CURRENT saved graph when no snapshot exists —
    /// a run that started before this snapshot landed, a resumed run
    /// (`flows_resume` re-enters an existing run without a fresh run-start),
    /// or a snapshot write that failed. That fallback is exactly the
    /// historical behaviour, so the degraded path is never worse than before.
    ///
    /// Returns an empty vec (logged, not propagated) when neither source
    /// yields anything — a flow deleted between run-finish and this handler
    /// firing, or a transient store error, both degrade to "nothing to
    /// settle" rather than panicking the event bus.
    fn dedup_node_ids(&self, namespace: &str, flow_id: &str, run_id: &str) -> Vec<String> {
        if let Some(ids) = self.snapshotted_dedup_node_ids(namespace, run_id) {
            tracing::trace!(
                target: "flows", %flow_id, %run_id, count = ids.len(),
                "[dedup-commit] settling against this run's dedup-node snapshot"
            );
            return ids;
        }

        match store::get_flow(&self.config, flow_id) {
            Ok(Some(flow)) => dedup_node_ids_in(&flow),
            Ok(None) => {
                tracing::debug!(target: "flows", %flow_id, "[dedup-commit] flow no longer exists — skipping");
                Vec::new()
            }
            Err(e) => {
                tracing::warn!(target: "flows", %flow_id, error = %e, "[dedup-commit] failed to load flow graph — skipping");
                Vec::new()
            }
        }
    }

    /// The dedup-node-id snapshot recorded for `run_id`, or `None` when this
    /// run has none — see [`dedup_node_ids`](Self::dedup_node_ids)'s fallback.
    ///
    /// An unreadable, non-array or empty snapshot is reported as absent so
    /// settlement degrades to the saved graph rather than to "settle
    /// nothing": settling nothing would strand tentative keys, whereas the
    /// saved graph is the behaviour this subsystem shipped with.
    fn snapshotted_dedup_node_ids(&self, namespace: &str, run_id: &str) -> Option<Vec<String>> {
        let key = run_dedup_snapshot_key(run_id);
        let value = match store::kv_get(&self.config, namespace, &key) {
            Ok(Some(value)) => value,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!(
                    target: "flows", %namespace, %run_id, error = %e,
                    "[dedup-commit] failed to read this run's dedup snapshot — falling back to \
                     the flow's saved graph"
                );
                return None;
            }
        };

        let ids: Vec<String> = value
            .as_array()?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        if ids.is_empty() {
            return None;
        }
        Some(ids)
    }

    /// Pins the ids of every `dedup` node in the graph this run is starting
    /// with, so [`Self::dedup_node_ids`] can settle the run against the nodes
    /// it actually executed rather than whatever the flow has been edited into
    /// by the time it finishes (issue #5268 item 2).
    ///
    /// Reading the saved graph here, rather than having `flows::ops` hand us
    /// its in-hand `Flow`, keeps the whole fix inside this subscriber.
    /// `FlowRunStarted` is published from `flows::ops::flows_run{,_detached}`
    /// immediately after the `flow_runs` row insert — before the engine
    /// executes a single node — so the graph read back here is the one the run
    /// is starting with.
    ///
    /// Best-effort like everything else in this subscriber: a load failure is
    /// logged and swallowed, degrading settlement to the historical "use the
    /// flow's current saved graph" behaviour rather than disturbing the run.
    fn snapshot_run_nodes(&self, flow_id: &str, run_id: &str) {
        match store::get_flow(&self.config, flow_id) {
            Ok(Some(flow)) => snapshot_run_dedup_nodes(&self.config, flow_id, run_id, &flow),
            // Nothing to pin, and not an error worth logging: the fallback
            // already handles a flow that is no longer there.
            Ok(None) => {}
            Err(e) => {
                tracing::warn!(
                    target: "flows", %flow_id, %run_id, error = %e,
                    "[dedup-commit] could not load flow to snapshot its dedup nodes at run start \
                     — settlement will fall back to the flow's saved graph"
                );
            }
        }
    }

    /// Drops this run's dedup-node snapshot once the run has been settled.
    ///
    /// Best-effort: a failed delete leaves one small run-scoped KV row
    /// behind, which nothing reads again (snapshots are keyed by `run_id`, and
    /// run ids are never reused).
    fn clear_run_snapshot(&self, namespace: &str, flow_id: &str, run_id: &str) {
        let key = run_dedup_snapshot_key(run_id);
        if let Err(e) = store::kv_delete(&self.config, namespace, &key) {
            tracing::warn!(
                target: "flows", %flow_id, %run_id, error = %e,
                "[dedup-commit] failed to clear this run's dedup snapshot — harmless: the key is \
                 run-scoped and is never read again"
            );
        }
    }

    async fn handle_finished(&self, flow_id: &str, run_id: &str, status: &str) {
        let namespace = flow_state_namespace(flow_id);
        let node_ids = self.dedup_node_ids(&namespace, flow_id, run_id);
        if node_ids.is_empty() {
            tracing::trace!(target: "flows", %flow_id, %run_id, %status, "[dedup-commit] no dedup nodes in this flow — nothing to settle");
            return;
        }

        let success = matches!(status, "completed" | "completed_with_warnings");
        tracing::debug!(
            target: "flows", %flow_id, %run_id, %status, success,
            dedup_node_count = node_ids.len(),
            "[dedup-commit] settling dedup nodes for finished run"
        );

        // Serialize this flow's settlement against any other overlapping
        // `FlowRunFinished` handling for the SAME flow_id — held across the
        // whole read-modify-write loop below so two overlapping runs can
        // never interleave their load(committed)+union(tentative)+
        // store(committed) and lose one run's keys. See `FLOW_COMMIT_LOCKS`
        // docs for the full race this closes.
        let lock = flow_commit_lock(flow_id);
        let lock_guard = lock.lock().await;
        tracing::trace!(target: "flows", %flow_id, %run_id, "[dedup-commit] acquired per-flow commit lock");
        self.maybe_test_delay().await;

        for node_id in node_ids {
            if success {
                self.commit(&namespace, &node_id, flow_id, run_id);
            } else {
                self.release(&namespace, &node_id, flow_id, run_id);
            }
        }

        // Still inside the per-flow lock: the snapshot is this run's alone, so
        // no other run contends for it, but keeping the delete here means a
        // settled run never leaves a readable snapshot behind for a
        // late-arriving duplicate `FlowRunFinished` to settle a second time.
        self.clear_run_snapshot(&namespace, flow_id, run_id);

        drop(lock_guard);
        tracing::trace!(target: "flows", %flow_id, %run_id, "[dedup-commit] released per-flow commit lock");
    }

    /// Success path: union this node's `tentative` set into `committed`, then
    /// clear `tentative`.
    fn commit(&self, namespace: &str, node_id: &str, flow_id: &str, run_id: &str) {
        let tentative_key = dedup_node::tentative_key(node_id);
        let committed_key = dedup_node::committed_key(node_id);

        let tentative = load_key_set(&self.config, namespace, &tentative_key);
        if tentative.is_empty() {
            tracing::trace!(target: "flows", %flow_id, %run_id, node_id, "[dedup-commit] no tentative keys — nothing to commit");
            return;
        }

        let mut committed = load_key_set(&self.config, namespace, &committed_key);
        let added = tentative
            .iter()
            .filter(|k| committed.insert((*k).clone()))
            .count();

        if let Err(e) = store_key_set(&self.config, namespace, &committed_key, &committed) {
            tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] failed to write committed set — tentative left in place, will \
                 retry the commit on this node's next successful run"
            );
            return;
        }
        tracing::debug!(
            target: "flows", %flow_id, %run_id, node_id, added, committed_len = committed.len(),
            "[dedup-commit] committed tentative keys"
        );

        if let Err(e) = store::kv_delete(&self.config, namespace, &tentative_key) {
            tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] committed but failed to clear tentative — harmless: the next \
                 run's dedup load will re-union the same, now-already-committed keys (committed \
                 is a set, so re-adding them is a no-op)"
            );
        }
    }

    /// Failure path: clear `tentative` only, leaving `committed` untouched so
    /// the released keys retry on the flow's next run.
    ///
    /// Deliberately does NOT `load_key_set` first to report a count: that
    /// would be a full `kv_get` + JSON deserialize + `HashSet` build purely
    /// for a log line, and `kv_delete` already silently no-ops on a missing
    /// key, so there is no early-return to save either (Greptile, issue
    /// #5265).
    fn release(&self, namespace: &str, node_id: &str, flow_id: &str, run_id: &str) {
        match store::kv_delete(&self.config, namespace, &dedup_node::tentative_key(node_id)) {
            Ok(()) => tracing::debug!(
                target: "flows", %flow_id, %run_id, node_id,
                "[dedup-commit] released tentative keys (if any) — will retry next run"
            ),
            Err(e) => tracing::warn!(
                target: "flows", %flow_id, %run_id, node_id, error = %e,
                "[dedup-commit] failed to release tentative — those keys remain tentative until \
                 a future successful commit reconciles them (harmless: committed stays untouched \
                 either way, so no item is ever wrongly marked done)"
            ),
        }
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for DedupCommitSubscriber {
    fn name(&self) -> &str {
        "flows::dedup_commit"
    }

    fn domains(&self) -> Option<&[&str]> {
        // Same reasoning as `FlowRunDigestSubscriber::domains` just above:
        // `FlowRunStarted` and `FlowRunFinished` are both tagged `"cron"` by
        // `DomainEvent::domain()` — they share a single match arm there.
        Some(&["cron"])
    }

    async fn handle(&self, event: &DomainEvent) {
        // Pin the `dedup` nodes THIS run is about to execute, so settlement at
        // `FlowRunFinished` uses the graph the run really ran rather than
        // whatever the flow has been edited into by the time it finishes
        // (issue #5268 item 2).
        if let DomainEvent::FlowRunStarted { flow_id, run_id } = event {
            self.snapshot_run_nodes(flow_id, run_id);
        }
        if let DomainEvent::FlowRunFinished {
            flow_id,
            run_id,
            status,
        } = event
        {
            self.handle_finished(flow_id, run_id, status).await;
        }
    }
}

/// The `StateStore` namespace a flow's `dedup` keys live in.
///
/// MUST match `tinyflows::caps::build_capabilities`'s `state_namespace`
/// (`src/openhuman/flows/tinyflows/caps.rs`) — colliding with the very keys
/// the engine's own `dedup` node reads and writes during the run is the entire
/// point. Deliberately NOT [`flow_namespace`], which is the flow's *memory*
/// namespace and a different string.
fn flow_state_namespace(flow_id: &str) -> String {
    format!("flow:{flow_id}")
}

/// KV key holding the dedup-node-id snapshot for a single run (issue #5268).
///
/// The `dedup::` prefix (double colon) cannot collide with the engine's own
/// `dedup:<node_id>:<committed|tentative>` keys — reaching this shape would
/// need a node id beginning with `:`, which the graph schema does not admit.
fn run_dedup_snapshot_key(run_id: &str) -> String {
    format!("dedup::run_nodes:{run_id}")
}

/// The ids of every `dedup` node in `flow`'s graph, in graph order.
fn dedup_node_ids_in(flow: &Flow) -> Vec<String> {
    let nodes = &flow.graph.nodes;
    nodes
        .iter()
        .filter(|n| n.kind == NodeKind::Dedup)
        .map(|n| n.id.clone())
        .collect()
}

/// Records the ids of every `dedup` node in the graph a run is about to
/// execute, so [`DedupCommitSubscriber`] settles that run against the graph it
/// really ran rather than whatever the flow has been edited into by the time it
/// finishes (issue #5268 item 2).
///
/// Called from [`DedupCommitSubscriber`]'s `FlowRunStarted` arm, which fires
/// from `flows::ops::flows_run{,_detached}` immediately after the initial
/// `flow_runs` row insert and before the engine executes any node.
///
/// Writes nothing when the graph has no `dedup` node — the overwhelming
/// majority of flows — so the common path costs one graph scan and zero I/O,
/// and an absent key stays an unambiguous "no snapshot" for
/// [`DedupCommitSubscriber::dedup_node_ids`]'s fallback.
///
/// Best-effort like the rest of this subsystem: a failed write is logged and
/// swallowed, degrading settlement to the historical
/// load-the-current-saved-graph behaviour rather than failing the run.
fn snapshot_run_dedup_nodes(config: &Config, flow_id: &str, run_id: &str, flow: &Flow) {
    let node_ids = dedup_node_ids_in(flow);
    if node_ids.is_empty() {
        return;
    }

    let namespace = flow_state_namespace(flow_id);
    let key = run_dedup_snapshot_key(run_id);
    let value = Value::Array(node_ids.into_iter().map(Value::String).collect());
    if let Err(e) = store::kv_set(config, &namespace, &key, &value) {
        tracing::warn!(
            target: "flows", %flow_id, %run_id, error = %e,
            "[dedup-commit] failed to snapshot this run's dedup nodes — settlement will fall back \
             to the flow's saved graph at finish time"
        );
    }
}

/// Loads a `dedup` node's key set (stored as a JSON array of strings) from
/// the flow-state KV table. Mirrors
/// `tinyflows::nodes::control_flow::dedup`'s own key-set loader: a missing
/// key, a non-array value, or an array with non-string elements all degrade
/// to an empty set rather than an error — a first run against a fresh store
/// has nothing recorded yet, which is not a fault.
fn load_key_set(config: &Config, namespace: &str, key: &str) -> HashSet<String> {
    match store::kv_get(config, namespace, key) {
        Ok(Some(value)) => value
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        Ok(None) => HashSet::new(),
        Err(e) => {
            tracing::warn!(target: "flows", %namespace, key, error = %e, "[dedup-commit] failed to load key set — treating as empty");
            HashSet::new()
        }
    }
}

/// Persists `set` under `key` as a JSON array of strings, sorted for a
/// stable, diffable on-disk representation (membership is exact-match either
/// way, so sort order carries no semantic meaning).
fn store_key_set(
    config: &Config,
    namespace: &str,
    key: &str,
    set: &HashSet<String>,
) -> anyhow::Result<()> {
    let mut keys: Vec<String> = set.iter().cloned().collect();
    keys.sort_unstable();
    let value = Value::Array(keys.into_iter().map(Value::String).collect());
    store::kv_set(config, namespace, key, &value)
}

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
