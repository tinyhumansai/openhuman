//! The background orchestrator runtime: fan-in → gate → queue → session.
//!
//! [`TriggerOrchestrator`] ties the slices together:
//! 1. `ingest` (called from the bus subscriber, non-blocking): normalize the
//!    event, run dedupe + rate limiting, and — for admitted triggers —
//!    spawn a gate task so the runtime never blocks event dispatch.
//! 2. the gate task runs the LLM gate; on `Promote` it pushes onto the
//!    [`OrchestratorQueue`] and wakes the loop.
//! 3. `run_loop` drains the queue serially, handing each promoted trigger to
//!    the [`LongLivedSession`].
//!
//! A process-global singleton holds the runtime so the bus subscriber and
//! the spawned loop share one instance.

use std::sync::{Arc, OnceLock};
use std::time::Instant;

use async_trait::async_trait;
use tokio::sync::Notify;
use tracing::{debug, info, warn};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::subconscious::LongLivedSession;

use super::gate::GatePass;
use super::normalize::normalize;
use super::queue::{EnqueueOutcome, OrchestratorQueue};
use super::registry::{AdmitOutcome, DedupeWindow, RateLimiter, TriggerRegistry};
use super::types::{GateDecision, Trigger};

/// The promote/drop gate. The production implementation is [`GatePass`]
/// (LLM-backed via `agent::triage`); tests inject a scripted gate so the
/// orchestration flow can be driven deterministically.
#[async_trait]
pub trait Gate: Send + Sync {
    async fn evaluate(&self, trigger: &Trigger, now: f64) -> GateDecision;
}

#[async_trait]
impl Gate for GatePass {
    async fn evaluate(&self, trigger: &Trigger, now: f64) -> GateDecision {
        GatePass::evaluate(self, trigger, now).await
    }
}

/// Executes a promoted trigger as a long-lived-session turn. The production
/// implementation is [`LongLivedSession`]; tests inject a scripted executor
/// to simulate agent / sub-agent behaviour and back-and-forth comms.
#[async_trait]
pub trait SessionExecutor: Send + Sync {
    /// Run the promoted trigger. `summary` is the synthesized user-turn;
    /// `external_content` marks third-party-tainted input. Returns the
    /// session's response text.
    async fn execute(&self, summary: &str, external_content: bool) -> Result<String, String>;

    /// Reserved thread id this executor writes to (for logging).
    fn thread_id(&self) -> &str;
}

#[async_trait]
impl SessionExecutor for LongLivedSession {
    async fn execute(&self, summary: &str, external_content: bool) -> Result<String, String> {
        self.process_promoted(summary, external_content)
            .await
            .map(|outcome| outcome.response)
    }

    fn thread_id(&self) -> &str {
        LongLivedSession::thread_id(self)
    }
}

/// Tunables for the trigger pipeline. Sourced from `HeartbeatConfig` in
/// slice 7; sensible defaults until then.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    pub queue_capacity: usize,
    pub dedupe_ttl_secs: f64,
    pub rate_capacity: f64,
    pub rate_refill_per_sec: f64,
    pub max_promotions_per_hour: u32,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        Self {
            queue_capacity: 256,
            dedupe_ttl_secs: 300.0,
            rate_capacity: 30.0,
            rate_refill_per_sec: 1.0,
            max_promotions_per_hour: 30,
        }
    }
}

/// Background orchestrator: trigger ingestion front-end + serial event loop.
pub struct TriggerOrchestrator {
    registry: TriggerRegistry,
    gate: Arc<dyn Gate>,
    queue: Arc<OrchestratorQueue>,
    session: Arc<dyn SessionExecutor>,
    notify: Arc<Notify>,
}

impl TriggerOrchestrator {
    /// Production constructor: real LLM gate ([`GatePass`]) over the given
    /// session executor (normally a [`LongLivedSession`]).
    pub fn new(session: Arc<dyn SessionExecutor>, config: OrchestratorConfig) -> Self {
        let gate = Arc::new(GatePass::new(config.max_promotions_per_hour));
        Self::with_components(session, gate, config)
    }

    /// Constructor with an injected [`Gate`] — used by tests to drive the
    /// orchestration flow deterministically without an LLM.
    pub fn with_components(
        session: Arc<dyn SessionExecutor>,
        gate: Arc<dyn Gate>,
        config: OrchestratorConfig,
    ) -> Self {
        let registry = TriggerRegistry::new(
            DedupeWindow::new(config.dedupe_ttl_secs),
            RateLimiter::new(config.rate_capacity, config.rate_refill_per_sec),
        );
        Self {
            registry,
            gate,
            queue: Arc::new(OrchestratorQueue::new(config.queue_capacity)),
            session,
            notify: Arc::new(Notify::new()),
        }
    }

    /// Non-blocking ingestion entry point for the bus subscriber. Normalizes
    /// + admits synchronously, then spawns the gate task for admitted
    /// triggers so event dispatch is never blocked on an LLM call.
    pub fn ingest(self: &Arc<Self>, event: &DomainEvent) {
        let now = now_secs();
        let Some(trigger) = normalize(event, now) else {
            return;
        };
        match self.registry.admit(&trigger, now) {
            AdmitOutcome::Admitted => {
                let this = Arc::clone(self);
                tokio::spawn(async move {
                    this.gate_and_enqueue(trigger).await;
                });
            }
            AdmitOutcome::Duplicate => {
                debug!(
                    "[subconscious_triggers] dropped duplicate trigger source={} key={}",
                    trigger.source.family(),
                    trigger.dedupe_key.as_str()
                );
            }
            AdmitOutcome::RateLimited => {
                debug!(
                    "[subconscious_triggers] rate-limited trigger source={}",
                    trigger.source.family()
                );
            }
        }
    }

    /// Run the gate on an admitted trigger and enqueue promotions.
    async fn gate_and_enqueue(self: Arc<Self>, trigger: super::types::Trigger) {
        let started = Instant::now();
        let now = now_secs();
        let source = trigger.source.family().to_string();
        let decision = self.gate.evaluate(&trigger, now).await;
        let latency_ms = started.elapsed().as_millis() as u64;
        let promoted = decision.is_promote();

        publish_global(DomainEvent::SubconsciousTriggerProcessed {
            source: source.clone(),
            decision: decision.as_str().to_string(),
            promoted,
            latency_ms,
        });

        match decision {
            GateDecision::Promote {
                synthesized_summary,
                priority,
                reason,
            } => {
                info!(
                    "[subconscious_triggers] promoting trigger source={} priority={} reason={}",
                    source,
                    priority.as_str(),
                    reason
                );
                // Reuse the trigger as the queue item: stamp the gate's
                // priority and carry the synthesized user-turn in
                // `gate_summary`. The gate only ever saw the *redacted*
                // summary; the long-lived session needs the actionable
                // content, so append a bounded rendering of the full payload
                // here. This is safe because promoted runs from external
                // triggers execute under the tainted automation origin (the
                // approval gate refuses external-effect tools).
                let mut item = trigger;
                item.priority = priority;
                item.payload.gate_summary =
                    augment_with_payload(&synthesized_summary, &item.payload.raw);
                match self.queue.push(item) {
                    EnqueueOutcome::Accepted => {}
                    EnqueueOutcome::EvictedLowest { evicted } => {
                        warn!(
                            "[subconscious_triggers] queue full — evicted lower-priority trigger {}",
                            evicted.display_label
                        );
                    }
                    EnqueueOutcome::DroppedIncoming => {
                        warn!(
                            "[subconscious_triggers] queue full — dropped promoted trigger source={source}"
                        );
                        return;
                    }
                }
                self.notify.notify_one();
            }
            GateDecision::Drop {
                acknowledge,
                reason,
            } => {
                debug!(
                    "[subconscious_triggers] dropped trigger source={source} ack={acknowledge} reason={reason}"
                );
            }
        }
    }

    /// Serial event loop: drain the queue, process each promoted trigger
    /// through the long-lived session, then wait for the next wake.
    ///
    /// Runs until the process exits.
    pub async fn run_loop(self: Arc<Self>) {
        info!(
            "[subconscious_triggers] orchestrator loop started thread={}",
            self.session.thread_id()
        );
        loop {
            while let Some(item) = self.queue.pop() {
                let summary = item.payload.gate_summary.clone();
                let external = item.external_content;
                debug!(
                    "[subconscious_triggers] processing promoted item source={} label={}",
                    item.source.family(),
                    item.display_label
                );
                if let Err(err) = self.session.execute(&summary, external).await {
                    warn!(
                        "[subconscious_triggers] session run failed label={} err={}",
                        item.display_label, err
                    );
                }
            }
            self.notify.notified().await;
        }
    }

    /// Test/diagnostic accessor for the pending queue depth.
    pub fn queue_depth(&self) -> usize {
        self.queue.len()
    }
}

/// Live orchestrator + the handle to its spawned event loop, so the loop can
/// be aborted on teardown (user switch).
struct OrchestratorSlot {
    orchestrator: Arc<TriggerOrchestrator>,
    loop_handle: tokio::task::JoinHandle<()>,
}

/// Process-global orchestrator slot. The `OnceLock` only initializes the
/// mutex; the contained `Option` is mutable so the orchestrator can be torn
/// down and re-bound across user/workspace switches.
static ORCHESTRATOR: OnceLock<std::sync::Mutex<Option<OrchestratorSlot>>> = OnceLock::new();

fn slot() -> &'static std::sync::Mutex<Option<OrchestratorSlot>> {
    ORCHESTRATOR.get_or_init(|| std::sync::Mutex::new(None))
}

/// Initialize the global orchestrator and spawn its event loop. Idempotent and
/// race-safe: the check-and-set happens under one lock, so a concurrent caller
/// never spawns a second loop. Returns the shared handle.
pub fn init_global(orch: Arc<TriggerOrchestrator>) -> Arc<TriggerOrchestrator> {
    let mut guard = slot().lock().expect("orchestrator slot poisoned");
    if let Some(existing) = guard.as_ref() {
        return Arc::clone(&existing.orchestrator);
    }
    let loop_handle = {
        let this = Arc::clone(&orch);
        tokio::spawn(async move { this.run_loop().await })
    };
    *guard = Some(OrchestratorSlot {
        orchestrator: Arc::clone(&orch),
        loop_handle,
    });
    orch
}

/// Get the global orchestrator if it has been initialized.
pub fn global() -> Option<Arc<TriggerOrchestrator>> {
    slot()
        .lock()
        .expect("orchestrator slot poisoned")
        .as_ref()
        .map(|s| Arc::clone(&s.orchestrator))
}

/// Tear down the global orchestrator: abort its event loop and clear the slot
/// so a subsequent [`init_global`] (e.g. after a user/workspace switch) binds a
/// fresh session instead of routing through the stale one.
pub fn shutdown_global() {
    if let Some(s) = slot().lock().expect("orchestrator slot poisoned").take() {
        s.loop_handle.abort();
        info!("[subconscious_triggers] orchestrator loop shut down");
    }
}

/// Max characters of the raw payload appended to a promoted session turn.
const PROMOTED_PAYLOAD_MAX_CHARS: usize = 4000;

/// Append a bounded rendering of the trigger's full payload to the gate's
/// synthesized summary so the long-lived session can act on the actual
/// content (subject/body/etc.) the redacted gate summary omitted. No-op for a
/// null payload.
fn augment_with_payload(summary: &str, raw: &serde_json::Value) -> String {
    if raw.is_null() {
        return summary.to_string();
    }
    let mut rendered = serde_json::to_string_pretty(raw).unwrap_or_else(|_| raw.to_string());
    if rendered.chars().count() > PROMOTED_PAYLOAD_MAX_CHARS {
        rendered = rendered.chars().take(PROMOTED_PAYLOAD_MAX_CHARS).collect();
        rendered.push_str("\n…(truncated)");
    }
    format!("{summary}\n\nTrigger payload:\n{rendered}")
}

/// Epoch seconds with sub-second precision.
fn now_secs() -> f64 {
    chrono::Utc::now().timestamp_millis() as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::schema::SubconsciousMode;
    use crate::openhuman::subconscious::LongLivedSession;
    use std::path::PathBuf;

    fn orchestrator() -> Arc<TriggerOrchestrator> {
        let session = Arc::new(LongLivedSession::new(
            PathBuf::from("/tmp/subconscious-triggers-test"),
            SubconsciousMode::Simple,
        ));
        Arc::new(TriggerOrchestrator::new(
            session,
            OrchestratorConfig::default(),
        ))
    }

    #[tokio::test]
    async fn ingest_ignores_unrelated_events() {
        let orch = orchestrator();
        // ChannelConnected is in a watched domain but is not a trigger source
        // → normalize returns None → no gate task, no enqueue.
        orch.ingest(&DomainEvent::ChannelConnected {
            channel: "slack".into(),
        });
        assert_eq!(orch.queue_depth(), 0);
    }

    #[tokio::test]
    async fn ingest_skips_self_authored_user_messages() {
        let orch = orchestrator();
        // A message the orchestrator itself emitted (anti self-trigger).
        orch.ingest(&DomainEvent::ChannelInboundMessage {
            event_name: "msg".into(),
            channel: "slack".into(),
            message: "proactive".into(),
            sender: Some(super::super::SUBCONSCIOUS_SENDER_MARKER.into()),
            reply_target: None,
            thread_ts: None,
            raw_data: serde_json::Value::Null,
        });
        assert_eq!(orch.queue_depth(), 0);
    }

    #[test]
    fn default_config_is_sane() {
        let c = OrchestratorConfig::default();
        assert!(c.queue_capacity > 0);
        assert!(c.dedupe_ttl_secs > 0.0);
        assert!(c.max_promotions_per_hour > 0);
    }
}
