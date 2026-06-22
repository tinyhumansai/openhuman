//! Multi-party conversation e2e: **human ↔ subconscious orchestrator ↔ sub-agent**.
//!
//! This drives the *real* [`TriggerOrchestrator`] (ingest → dedupe/rate →
//! gate → priority queue → serial run loop) with two injected seams — a
//! scripted [`Gate`] and a scripted [`SessionExecutor`] — so we can simulate
//! many back-and-forth comms patterns deterministically, without an LLM:
//!
//! - **human → subconscious**: a `ChannelInboundMessage` is normalized,
//!   gated, promoted, and run.
//! - **subconscious → sub-agent**: the scripted session "spawns" a sub-agent
//!   by emitting a follow-on `SubagentCompleted`/`SubagentFailed` event that
//!   re-enters the real pipeline.
//! - **sub-agent → subconscious**: that conclusion is normalized, gated, and
//!   merged by the session.
//! - **subconscious → human**: the session calls the real `notify_user`,
//!   which we capture off the event bus.
//!
//! Everything between the two seams is the production code path: event
//! normalization, the dedupe window, per-source rate limiting, the promotion
//! gate budget, the priority queue, the serial loop, and the anti-self-trigger
//! guard. The harness re-injects emitted follow-on events through the same
//! `ingest` entry point the bus subscriber uses, so cascades are exercised
//! exactly as in production.
//!
//! Run with a visible transcript:
//! `cargo test --test subconscious_conversation_e2e -- --nocapture`

use std::collections::VecDeque;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;

use openhuman_core::core::event_bus::{global, init_global, DomainEvent};
use openhuman_core::openhuman::subconscious_triggers::types::{
    GateDecision, Trigger, TriggerPriority, TriggerSource,
};
use openhuman_core::openhuman::subconscious_triggers::{
    Gate, OrchestratorConfig, SessionExecutor, TriggerOrchestrator,
};

// ─────────────────────────────────────────────────────────────────────────────
// Shared conversation transcript — the record of "what happened".
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct Transcript(Arc<StdMutex<Vec<String>>>);

impl Transcript {
    fn new() -> Self {
        Self(Arc::new(StdMutex::new(Vec::new())))
    }
    fn push(&self, line: impl Into<String>) {
        self.0.lock().unwrap().push(line.into());
    }
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
    fn count(&self, needle: &str) -> usize {
        self.0
            .lock()
            .unwrap()
            .iter()
            .filter(|l| l.contains(needle))
            .count()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scripted gate — promote/drop driven by trigger source + content rules.
// ─────────────────────────────────────────────────────────────────────────────

struct ScriptedGate {
    transcript: Transcript,
}

#[async_trait]
impl Gate for ScriptedGate {
    async fn evaluate(&self, trigger: &Trigger, _now: f64) -> GateDecision {
        let summary = &trigger.payload.gate_summary;
        // Routine cron noise is dropped.
        let drop =
            matches!(trigger.source, TriggerSource::Cron { .. }) || summary.contains("[ignore]");
        if drop {
            self.transcript
                .push(format!("GATE drop     {}", trigger.display_label));
            return GateDecision::Drop {
                acknowledge: false,
                reason: "routine/no-op".into(),
            };
        }
        // Everything else promotes. Urgent if the human flagged it.
        let priority = if summary.to_lowercase().contains("urgent") {
            TriggerPriority::Urgent
        } else {
            trigger.priority
        };
        self.transcript.push(format!(
            "GATE promote  {} (prio={})",
            trigger.display_label,
            priority.as_str()
        ));
        GateDecision::Promote {
            // Keep the original summary so the session can branch on it.
            synthesized_summary: summary.clone(),
            priority,
            reason: "actionable".into(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scripted session — models the subconscious agent's behaviour, including
// spawning sub-agents (via emitted follow-on events) and notifying the human.
// ─────────────────────────────────────────────────────────────────────────────

/// A follow-on event the scripted session wants to feed back into the
/// orchestrator (simulating the bus fan-in for sub-agent conclusions).
type Emitter = Arc<StdMutex<VecDeque<DomainEvent>>>;

struct ScriptedSession {
    transcript: Transcript,
    workspace: std::path::PathBuf,
    emit: Emitter,
    /// Monotonic id for spawned sub-agent tasks.
    next_task: Arc<std::sync::atomic::AtomicU64>,
}

impl ScriptedSession {
    fn emit(&self, event: DomainEvent) {
        self.emit.lock().unwrap().push_back(event);
    }
    fn spawn_subagent(&self, agent_id: &str, ok: bool) -> String {
        let n = self
            .next_task
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let task_id = format!("task-{n}");
        self.transcript
            .push(format!("SESSION spawn  sub-agent '{agent_id}' ({task_id})"));
        let event = if ok {
            DomainEvent::SubagentCompleted {
                parent_session: "subconscious:orchestrator".into(),
                task_id: task_id.clone(),
                agent_id: agent_id.into(),
                elapsed_ms: 5,
                output_chars: 120,
                iterations: 2,
            }
        } else {
            DomainEvent::SubagentFailed {
                parent_session: "subconscious:orchestrator".into(),
                task_id: task_id.clone(),
                agent_id: agent_id.into(),
                error: "tool timeout".into(),
            }
        };
        self.emit(event);
        task_id
    }
}

#[async_trait]
impl SessionExecutor for ScriptedSession {
    async fn execute(&self, summary: &str, _external: bool) -> Result<String, String> {
        let s = summary.to_lowercase();

        // Branch on what the promoted turn is about — modelling the
        // orchestrator agent's decisions. ORDER MATTERS: sub-agent conclusion
        // summaries mention the agent id ("researcher" ⊃ "research"), so the
        // conclusion branches must be checked before the delegate branch.
        if s.contains("failed") {
            // A sub-agent failed → recover, escalate a retry sub-agent.
            self.transcript
                .push("SESSION handle sub-agent FAILURE → retry");
            self.spawn_subagent("researcher", true);
            Ok("Retrying after failure.".into())
        } else if s.contains("completed in") || s.contains("chars of output") {
            // A sub-agent conclusion came back → merge + tell the human.
            self.transcript
                .push("SESSION merge  sub-agent conclusion → notifying human");
            openhuman_core::openhuman::subconscious::notify_user(
                self.workspace.clone(),
                "Your research is ready — here's the summary.",
                Some("research done"),
            );
            self.transcript.push("SESSION notify human (done)");
            Ok("Merged conclusion; user notified.".into())
        } else if s.contains("research") || s.contains("prep") || s.contains("deck") {
            // Human asked for deep work → delegate to a sub-agent.
            self.transcript
                .push(format!("SESSION run    (delegating) :: {summary}"));
            self.spawn_subagent("researcher", true);
            Ok("Delegated to researcher.".into())
        } else if s.contains("status") || s.contains("what") {
            // Simple Q → answer the human directly, no sub-agent.
            self.transcript
                .push(format!("SESSION reply  directly :: {summary}"));
            openhuman_core::openhuman::subconscious::notify_user(
                self.workspace.clone(),
                "Here's your status.",
                None,
            );
            Ok("Answered directly.".into())
        } else {
            self.transcript.push(format!("SESSION note   :: {summary}"));
            Ok("Noted.".into())
        }
    }

    fn thread_id(&self) -> &str {
        "subconscious:orchestrator"
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Harness.
// ─────────────────────────────────────────────────────────────────────────────

/// Serializes tests that touch the process-global event bus.
fn bus_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

struct Harness {
    orch: Arc<TriggerOrchestrator>,
    transcript: Transcript,
    emit: Emitter,
    notifications: Arc<StdMutex<Vec<String>>>,
    _loop: tokio::task::JoinHandle<()>,
    _sub: openhuman_core::core::event_bus::SubscriptionHandle,
    _tmp: tempfile::TempDir,
}

impl Harness {
    fn new(config: OrchestratorConfig) -> Self {
        init_global(128);
        let transcript = Transcript::new();
        let emit: Emitter = Arc::new(StdMutex::new(VecDeque::new()));
        let tmp = tempfile::tempdir().expect("tempdir");

        let session = Arc::new(ScriptedSession {
            transcript: transcript.clone(),
            workspace: tmp.path().to_path_buf(),
            emit: Arc::clone(&emit),
            next_task: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        });
        let gate = Arc::new(ScriptedGate {
            transcript: transcript.clone(),
        });
        let orch = Arc::new(TriggerOrchestrator::with_components(session, gate, config));

        // Capture proactive (subconscious → human) deliveries off the bus.
        let notifications = Arc::new(StdMutex::new(Vec::<String>::new()));
        let sink = Arc::clone(&notifications);
        let sub = global().expect("bus").on("conv-e2e-notify", move |event| {
            let sink = Arc::clone(&sink);
            let event = event.clone();
            Box::pin(async move {
                if let DomainEvent::ProactiveMessageRequested {
                    source, message, ..
                } = &event
                {
                    if source == "subconscious" {
                        sink.lock().unwrap().push(message.clone());
                    }
                }
            })
        });

        let loop_handle = Arc::clone(&orch);
        let task = tokio::spawn(async move { loop_handle.run_loop().await });

        Self {
            orch,
            transcript,
            emit,
            notifications,
            _loop: task,
            _sub: sub,
            _tmp: tmp,
        }
    }

    /// Feed an inbound event into the orchestrator (as the bus subscriber would).
    fn ingest(&self, event: DomainEvent) {
        self.orch.ingest(&event);
    }

    /// Pump the cascade until quiescent: repeatedly drain follow-on events the
    /// scripted session emitted back into `ingest`, waiting for the queue +
    /// gate tasks to settle between rounds. Bounded so a runaway loop fails
    /// fast instead of hanging.
    async fn settle(&self) {
        for _ in 0..200 {
            // Let in-flight gate tasks + the run loop make progress.
            tokio::time::sleep(Duration::from_millis(10)).await;
            let pending: Vec<DomainEvent> = {
                let mut q = self.emit.lock().unwrap();
                q.drain(..).collect()
            };
            for ev in &pending {
                self.orch.ingest(ev);
            }
            // Quiescent when nothing is queued and nothing new was emitted.
            if pending.is_empty() && self.orch.queue_depth() == 0 {
                // One more grace round to catch a just-finished session run
                // that emitted a trailing event.
                tokio::time::sleep(Duration::from_millis(15)).await;
                if self.emit.lock().unwrap().is_empty() && self.orch.queue_depth() == 0 {
                    return;
                }
            }
        }
        panic!("conversation did not settle — possible runaway cascade");
    }

    fn notifications(&self) -> Vec<String> {
        self.notifications.lock().unwrap().clone()
    }

    fn print(&self, title: &str) {
        println!("\n=== {title} ===");
        for line in self.transcript.lines() {
            println!("  {line}");
        }
        for n in self.notifications() {
            println!("  >> to human: {n}");
        }
    }
}

fn human_msg(channel: &str, sender: &str, message: &str) -> DomainEvent {
    DomainEvent::ChannelInboundMessage {
        event_name: "msg".into(),
        channel: channel.into(),
        message: message.into(),
        sender: Some(sender.into()),
        reply_target: Some("dm".into()),
        thread_ts: Some(format!("t-{}", message.len())),
        raw_data: serde_json::Value::Null,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 1 — full round trip: human → session → sub-agent → session → human.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn conversation_human_delegates_then_subagent_reports_back() {
    let _g = bus_lock();
    let h = Harness::new(OrchestratorConfig::default());

    // Human asks for deep work.
    h.ingest(human_msg(
        "slack",
        "U1",
        "can you prep the Q3 research deck?",
    ));
    h.settle().await;
    h.print("scenario 1: delegate → sub-agent → report back");

    // The session delegated (spawned a sub-agent) …
    assert_eq!(
        h.transcript.count("spawn  sub-agent"),
        1,
        "one sub-agent spawned"
    );
    // … the conclusion came back through the real pipeline and was merged …
    assert_eq!(h.transcript.count("merge  sub-agent conclusion"), 1);
    // … and the human was notified exactly once.
    let notes = h.notifications();
    assert_eq!(notes.len(), 1, "exactly one human notification");
    assert!(notes[0].contains("research is ready"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 2 — sub-agent failure → recovery → retry → success → human.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn conversation_subagent_failure_recovers_with_retry() {
    let _g = bus_lock();
    let h = Harness::new(OrchestratorConfig::default());

    // Inject a sub-agent FAILURE conclusion directly (as if a prior spawn failed).
    h.ingest(DomainEvent::SubagentFailed {
        parent_session: "subconscious:orchestrator".into(),
        task_id: "task-0".into(),
        agent_id: "researcher".into(),
        error: "tool timeout".into(),
    });
    h.settle().await;
    h.print("scenario 2: failure → retry → success → human");

    // Failure handled, a retry sub-agent spawned, its success merged, human told.
    assert_eq!(h.transcript.count("handle sub-agent FAILURE"), 1);
    assert_eq!(h.transcript.count("spawn  sub-agent"), 1, "one retry spawn");
    assert_eq!(h.transcript.count("merge  sub-agent conclusion"), 1);
    assert_eq!(h.notifications().len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 3 — interleaved traffic: cron noise dropped, two humans, dedupe.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn conversation_interleaved_traffic_is_handled() {
    let _g = bus_lock();
    let h = Harness::new(OrchestratorConfig::default());

    // Routine cron tick — gate should drop it (no session run).
    h.ingest(DomainEvent::CronJobTriggered {
        job_id: "nightly".into(),
        job_name: "nightly recap".into(),
        job_type: "agent".into(),
    });
    // Human A asks a simple question → direct reply, no sub-agent.
    h.ingest(human_msg("slack", "U1", "what's my status today?"));
    // Human B duplicate-sends the exact same message twice (transport retry).
    let dup = human_msg("discord", "U2", "please prep the deck");
    h.ingest(dup.clone());
    h.ingest(dup);
    h.settle().await;
    h.print("scenario 3: interleaved cron + two humans + dedupe");

    // Cron was dropped (no session run for it).
    assert_eq!(h.transcript.count("GATE drop"), 1);
    // Human A answered directly (no sub-agent for a status question).
    assert_eq!(h.transcript.count("reply  directly"), 1);
    // Human B's duplicate collapsed → only ONE deck delegation despite two sends.
    assert_eq!(
        h.transcript.count("(delegating)"),
        1,
        "duplicate human msg collapsed"
    );
    // Two human-facing notifications: status answer + research-ready.
    assert_eq!(h.notifications().len(), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Scenario 4 — back-pressure: promotion budget caps a burst of human asks.
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn conversation_promotion_budget_caps_a_burst() {
    let _g = bus_lock();
    // The scripted gate intentionally has no promotion budget (that lives in
    // the real GatePass), so this scenario isolates the *rate limiter*: a
    // flood of distinct user messages from one source is capped by the token
    // bucket (capacity 3, no refill).
    let config = OrchestratorConfig {
        rate_capacity: 3.0,
        rate_refill_per_sec: 0.0,
        ..OrchestratorConfig::default()
    };
    let h = Harness::new(config);

    for i in 0..6 {
        h.ingest(human_msg("slack", "U1", &format!("status check #{i}")));
    }
    h.settle().await;
    h.print("scenario 4: rate-limited burst of human messages");

    // Only 3 of the 6 distinct messages passed the rate limiter → 3 replies.
    assert_eq!(
        h.transcript.count("reply  directly"),
        3,
        "rate limiter capped the burst"
    );
    assert_eq!(h.notifications().len(), 3);
}
