//! End-to-end scenario simulation for the subconscious **trigger pipeline**.
//!
//! This exercises the real public pipeline functions wired together —
//! `normalize` → `TriggerRegistry::admit` (dedupe + rate) → gate mapping
//! (`map_triage_to_gate` + `apply_budget`) → `OrchestratorQueue` — across
//! every trigger source and gate outcome, plus the real `notify_user`
//! handoff (event bus + reserved-thread persistence) and the reserved-thread
//! cold-boot contract the long-lived session depends on.
//!
//! It is hermetic (no network, no model) and deterministic: the LLM gate is
//! simulated by feeding the real `map_triage_to_gate` a `TriageDecision` for
//! each `TriageAction`, so we test *our* promote/drop/dedupe/budget/queue
//! logic exhaustively. The actual triage model call and the full agent turn
//! run through the native bus + global provider and are covered by the
//! `agent::triage` and agent-harness test suites.
//!
//! Run with a visible trace:
//! `cargo test --test subconscious_triggers_e2e -- --nocapture`

use std::sync::{Arc, Mutex as StdMutex, OnceLock};

use openhuman_core::core::event_bus::{global, init_global, DomainEvent};
use openhuman_core::openhuman::agent::triage::{TriageAction, TriageDecision};
use openhuman_core::openhuman::subconscious::{
    notify_user, ORCHESTRATOR_THREAD_ID, USER_THREAD_ID,
};
use openhuman_core::openhuman::subconscious_triggers::gate::{apply_budget, map_triage_to_gate};
use openhuman_core::openhuman::subconscious_triggers::{
    normalize, AdmitOutcome, DedupeWindow, EnqueueOutcome, GateDecision, OrchestratorQueue,
    PromotionBudget, RateLimiter, Trigger, TriggerPriority, TriggerRegistry, TriggerSource,
};

// ─────────────────────────────────────────────────────────────────────────────
// Event constructors for the four v1 trigger sources.
// ─────────────────────────────────────────────────────────────────────────────

fn cron_event(job_id: &str, name: &str) -> DomainEvent {
    DomainEvent::CronJobTriggered {
        job_id: job_id.into(),
        job_name: name.into(),
        job_type: "agent".into(),
    }
}

fn user_event(channel: &str, sender: Option<&str>, message: &str) -> DomainEvent {
    DomainEvent::ChannelInboundMessage {
        event_name: "msg".into(),
        channel: channel.into(),
        message: message.into(),
        sender: sender.map(str::to_string),
        reply_target: Some("dm".into()),
        thread_ts: Some("t1".into()),
        raw_data: serde_json::Value::Null,
    }
}

fn composio_event(metadata_id: &str, subject: &str) -> DomainEvent {
    DomainEvent::ComposioTriggerReceived {
        toolkit: "gmail".into(),
        trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        metadata_id: metadata_id.into(),
        metadata_uuid: format!("{metadata_id}-uuid"),
        payload: serde_json::json!({ "subject": subject, "body": "PRIVATE BODY" }),
    }
}

fn subagent_done_event(task_id: &str, agent_id: &str) -> DomainEvent {
    DomainEvent::SubagentCompleted {
        parent_session: "subconscious:orchestrator".into(),
        task_id: task_id.into(),
        agent_id: agent_id.into(),
        elapsed_ms: 10,
        output_chars: 100,
        iterations: 2,
    }
}

fn triage(action: TriageAction, prompt: Option<&str>) -> TriageDecision {
    TriageDecision {
        action,
        target_agent: prompt.map(|_| "orchestrator".into()),
        prompt: prompt.map(str::to_string),
        reason: "simulated gate verdict".into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 1 — normalization: every source maps with the right shape.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn scenario_normalization_covers_all_sources() {
    let now = 100.0;

    let cron = normalize(&cron_event("j1", "morning brief"), now).expect("cron normalizes");
    assert_eq!(cron.priority, TriggerPriority::Low);
    assert!(!cron.external_content);
    assert!(matches!(cron.source, TriggerSource::Cron { .. }));

    let user = normalize(&user_event("slack", Some("U1"), "what's on my plate?"), now)
        .expect("user normalizes");
    assert_eq!(user.priority, TriggerPriority::High);
    assert!(
        user.external_content,
        "inbound channel messages are untrusted"
    );
    assert!(user.payload.gate_summary.contains("what's on my plate"));

    let composio =
        normalize(&composio_event("evt-1", "Invoice #42"), now).expect("composio normalizes");
    assert_eq!(composio.priority, TriggerPriority::Normal);
    assert!(composio.external_content, "third-party content is tainted");
    // Redaction: the private body never reaches the gate summary.
    assert!(!composio.payload.gate_summary.contains("PRIVATE BODY"));
    // …but is retained in raw for promotion synthesis.
    assert_eq!(composio.payload.raw["payload"]["body"], "PRIVATE BODY");

    let subagent =
        normalize(&subagent_done_event("task-1", "researcher"), now).expect("subagent normalizes");
    assert!(matches!(
        subagent.source,
        TriggerSource::SubagentConclusion { ok: true, .. }
    ));

    // A self-authored proactive message must NOT become a trigger (anti-loop).
    assert!(
        normalize(&user_event("slack", Some("subconscious"), "proactive"), now).is_none(),
        "orchestrator's own output must not re-trigger it"
    );

    // Unrelated events are ignored.
    assert!(normalize(
        &DomainEvent::ChannelConnected {
            channel: "slack".into()
        },
        now
    )
    .is_none());
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 2 — admission: dedupe collapses storms, rate limits floods.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn scenario_dedupe_collapses_webhook_storm() {
    let reg = TriggerRegistry::with_defaults();
    // 50 identical Gmail webhooks (same metadata_id) within the TTL window.
    let mut admitted = 0;
    let mut duplicates = 0;
    for i in 0..50 {
        let ev = composio_event("same-evt", "dup");
        let t = normalize(&ev, 1000.0 + i as f64 * 0.01).expect("normalize");
        match reg.admit(&t, 1000.0 + i as f64 * 0.01) {
            AdmitOutcome::Admitted => admitted += 1,
            AdmitOutcome::Duplicate => duplicates += 1,
            AdmitOutcome::RateLimited => {}
        }
    }
    assert_eq!(admitted, 1, "only the first of the storm is admitted");
    assert_eq!(duplicates, 49);
}

#[test]
fn scenario_rate_limit_caps_distinct_events_per_source() {
    // Tiny bucket: capacity 2, no refill, so the 3rd distinct event of the
    // same family is rate-limited even though it passes dedupe.
    let reg = TriggerRegistry::new(DedupeWindow::new(600.0), RateLimiter::new(2.0, 0.0));
    let outcomes: Vec<AdmitOutcome> = (0..3)
        .map(|i| {
            let ev = composio_event(&format!("evt-{i}"), "x");
            let t = normalize(&ev, 5.0).expect("normalize");
            reg.admit(&t, 5.0)
        })
        .collect();
    assert_eq!(outcomes[0], AdmitOutcome::Admitted);
    assert_eq!(outcomes[1], AdmitOutcome::Admitted);
    assert_eq!(outcomes[2], AdmitOutcome::RateLimited);
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 3 — gate decisions: every triage action maps correctly.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn scenario_gate_maps_every_triage_action() {
    let now = 0.0;
    let budget = PromotionBudget::new(100);
    let base = normalize(&composio_event("g1", "subj"), now).expect("normalize");

    // Drop → dropped, not acknowledged.
    let d = apply_budget(
        map_triage_to_gate(&triage(TriageAction::Drop, None), &base),
        &budget,
        now,
    );
    assert!(matches!(
        d,
        GateDecision::Drop {
            acknowledge: false,
            ..
        }
    ));

    // Acknowledge → dropped but acknowledged.
    let a = apply_budget(
        map_triage_to_gate(&triage(TriageAction::Acknowledge, None), &base),
        &budget,
        now,
    );
    assert!(matches!(
        a,
        GateDecision::Drop {
            acknowledge: true,
            ..
        }
    ));

    // React → promote, keeping the trigger's own priority.
    let r = map_triage_to_gate(&triage(TriageAction::React, Some("ack it")), &base);
    match r {
        GateDecision::Promote {
            priority,
            synthesized_summary,
            ..
        } => {
            assert_eq!(priority, base.priority);
            assert!(synthesized_summary.contains("ack it"));
        }
        other => panic!("expected promote, got {other:?}"),
    }

    // Escalate → promote at >= High.
    let e = map_triage_to_gate(&triage(TriageAction::Escalate, Some("draft reply")), &base);
    match e {
        GateDecision::Promote { priority, .. } => assert!(priority >= TriggerPriority::High),
        other => panic!("expected promote, got {other:?}"),
    }
}

#[test]
fn scenario_promotion_budget_exhaustion_downgrades_to_ack_drop() {
    let now = 0.0;
    let budget = PromotionBudget::new(1); // one promotion per hour
    let base = normalize(&user_event("slack", Some("U1"), "urgent!"), now).expect("normalize");
    let escalate = || map_triage_to_gate(&triage(TriageAction::Escalate, Some("do it")), &base);

    // First escalation promotes.
    assert!(apply_budget(escalate(), &budget, now).is_promote());
    // Second within the hour is downgraded to an acknowledged drop — noted,
    // but no reasoning-tier session run is spent.
    match apply_budget(escalate(), &budget, now) {
        GateDecision::Drop {
            acknowledge,
            reason,
        } => {
            assert!(acknowledge);
            assert!(reason.contains("budget exhausted"));
        }
        other => panic!("expected budget downgrade, got {other:?}"),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 4 — queue: priority ordering + overflow eviction.
// ─────────────────────────────────────────────────────────────────────────────

fn promoted(label: &str, priority: TriggerPriority) -> Trigger {
    let mut t = normalize(&cron_event("j", label), 0.0).expect("normalize");
    t.display_label = label.into();
    t.priority = priority;
    t
}

#[test]
fn scenario_queue_serves_highest_priority_first() {
    let q = OrchestratorQueue::new(16);
    q.push(promoted("cron-low", TriggerPriority::Low));
    q.push(promoted("user-high", TriggerPriority::High));
    q.push(promoted("webhook-normal", TriggerPriority::Normal));
    q.push(promoted("interrupt-urgent", TriggerPriority::Urgent));

    let drained: Vec<String> = std::iter::from_fn(|| q.pop())
        .map(|t| t.display_label)
        .collect();
    assert_eq!(
        drained,
        vec![
            "interrupt-urgent",
            "user-high",
            "webhook-normal",
            "cron-low"
        ]
    );
}

#[test]
fn scenario_queue_overflow_sheds_lowest_priority() {
    let q = OrchestratorQueue::new(2);
    assert_eq!(
        q.push(promoted("low", TriggerPriority::Low)),
        EnqueueOutcome::Accepted
    );
    assert_eq!(
        q.push(promoted("normal", TriggerPriority::Normal)),
        EnqueueOutcome::Accepted
    );
    // Full; an Urgent arrival evicts the lowest-priority held item.
    match q.push(promoted("urgent", TriggerPriority::Urgent)) {
        EnqueueOutcome::EvictedLowest { evicted } => assert_eq!(evicted.display_label, "low"),
        other => panic!("expected eviction, got {other:?}"),
    }
    // Full again; a Low arrival is dropped rather than evicting a better item.
    assert_eq!(
        q.push(promoted("late-low", TriggerPriority::Low)),
        EnqueueOutcome::DroppedIncoming
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 5 — full pipeline simulation across scenarios, with a printed trace.
// ─────────────────────────────────────────────────────────────────────────────

/// One scenario: a raw event + the gate verdict the (simulated) LLM returns.
struct Scenario {
    name: &'static str,
    event: DomainEvent,
    action: TriageAction,
    prompt: Option<&'static str>,
}

/// Outcome of running a scenario through the whole pipeline.
#[derive(Debug)]
enum Outcome {
    Ignored,
    Duplicate,
    RateLimited,
    Dropped,
    Promoted { priority: TriggerPriority },
}

/// Drive one event through normalize → admit → gate → enqueue, returning what
/// happened and (on promotion) pushing onto `queue`.
fn run_scenario(
    s: &Scenario,
    reg: &TriggerRegistry,
    budget: &PromotionBudget,
    queue: &OrchestratorQueue,
    now: f64,
) -> Outcome {
    let Some(trigger) = normalize(&s.event, now) else {
        return Outcome::Ignored;
    };
    match reg.admit(&trigger, now) {
        AdmitOutcome::Duplicate => return Outcome::Duplicate,
        AdmitOutcome::RateLimited => return Outcome::RateLimited,
        AdmitOutcome::Admitted => {}
    }
    let decision = apply_budget(
        map_triage_to_gate(&triage(s.action, s.prompt), &trigger),
        budget,
        now,
    );
    match decision {
        GateDecision::Drop { .. } => Outcome::Dropped,
        GateDecision::Promote {
            priority,
            synthesized_summary,
            ..
        } => {
            let mut item = trigger;
            item.priority = priority;
            item.payload.gate_summary = synthesized_summary;
            queue.push(item);
            Outcome::Promoted { priority }
        }
    }
}

#[test]
fn scenario_full_pipeline_simulation_with_trace() {
    let reg = TriggerRegistry::with_defaults();
    let budget = PromotionBudget::new(2); // tight budget to exercise exhaustion
    let queue = OrchestratorQueue::new(64);

    let scenarios = vec![
        Scenario {
            name: "cron tick — gate drops routine noise",
            event: cron_event("nightly", "nightly recap"),
            action: TriageAction::Drop,
            prompt: None,
        },
        Scenario {
            name: "user message — gate escalates (promote #1)",
            event: user_event("slack", Some("U1"), "can you prep the Q3 deck?"),
            action: TriageAction::Escalate,
            prompt: Some("prepare the Q3 deck"),
        },
        Scenario {
            name: "composio gmail — gate reacts (promote #2)",
            event: composio_event("inv-1", "Invoice overdue"),
            action: TriageAction::React,
            prompt: Some("flag the overdue invoice"),
        },
        Scenario {
            name: "composio gmail DUPLICATE — collapsed by dedupe",
            event: composio_event("inv-1", "Invoice overdue"),
            action: TriageAction::React,
            prompt: Some("flag the overdue invoice"),
        },
        Scenario {
            name: "subagent conclusion — would promote but budget exhausted",
            event: subagent_done_event("task-9", "researcher"),
            action: TriageAction::Escalate,
            prompt: Some("merge the research findings"),
        },
        Scenario {
            name: "self-authored proactive echo — ignored (anti-loop)",
            event: user_event("slack", Some("subconscious"), "FYI: deck is ready"),
            action: TriageAction::Escalate,
            prompt: Some("should never run"),
        },
    ];

    println!("\n=== subconscious trigger pipeline — scenario trace ===");
    let mut promotions = 0;
    let mut trace = Vec::new();
    for (i, s) in scenarios.iter().enumerate() {
        let outcome = run_scenario(s, &reg, &budget, &queue, 1000.0 + i as f64);
        if matches!(outcome, Outcome::Promoted { .. }) {
            promotions += 1;
        }
        println!("  [{i}] {:<55} -> {outcome:?}", s.name);
        trace.push(outcome);
    }
    println!("  queue depth after gating: {}", queue.len());
    println!("=== drain (highest priority first) ===");
    while let Some(item) = queue.pop() {
        println!(
            "  run -> {:<24} priority={}",
            item.display_label,
            item.priority.as_str()
        );
    }

    // Assertions on what the pipeline decided.
    assert!(matches!(trace[0], Outcome::Dropped), "cron noise dropped");
    assert!(
        matches!(trace[1], Outcome::Promoted { priority } if priority >= TriggerPriority::High),
        "user escalation promoted at >= High"
    );
    assert!(
        matches!(trace[2], Outcome::Promoted { .. }),
        "gmail react promoted"
    );
    assert!(
        matches!(trace[3], Outcome::Duplicate),
        "duplicate gmail collapsed"
    );
    assert!(
        matches!(trace[4], Outcome::Dropped),
        "budget (2) exhausted → 3rd promotion downgraded to drop"
    );
    assert!(matches!(trace[5], Outcome::Ignored), "self-echo ignored");
    assert_eq!(promotions, 2, "exactly two promotions within the budget");
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 6 — notify_user: real event bus + reserved user-thread persistence.
// ─────────────────────────────────────────────────────────────────────────────

/// Serializes tests that touch the process-global event bus so concurrent
/// captures don't cross-talk.
fn bus_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: OnceLock<StdMutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| StdMutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

#[tokio::test]
async fn scenario_notify_user_delivers_and_persists() {
    let _guard = bus_lock();
    init_global(64);

    let captured: Arc<StdMutex<Vec<DomainEvent>>> = Arc::new(StdMutex::new(Vec::new()));
    let sink = Arc::clone(&captured);
    let _sub = global()
        .expect("bus initialized")
        .on("e2e-notify-capture", move |event| {
            let sink = Arc::clone(&sink);
            let event = event.clone();
            Box::pin(async move {
                if let DomainEvent::ProactiveMessageRequested { .. } = &event {
                    sink.lock().unwrap().push(event);
                }
            })
        });

    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().to_path_buf();

    let unique = "E2E-NOTIFY-MARKER-7321";
    notify_user(workspace.clone(), unique, Some("heads-up"));

    // Give the async broadcast a moment to deliver.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 1) The proactive delivery event fired, tagged as subconscious-sourced.
    let events = captured.lock().unwrap();
    let found = events.iter().any(|e| match e {
        DomainEvent::ProactiveMessageRequested {
            source, message, ..
        } => source == "subconscious" && message == unique,
        _ => false,
    });
    assert!(
        found,
        "notify_user must publish a ProactiveMessageRequested event"
    );

    // 2) The message landed in the reserved user-facing thread.
    let persisted =
        openhuman_core::openhuman::memory_conversations::get_messages(workspace, USER_THREAD_ID)
            .expect("read user thread");
    assert!(
        persisted
            .iter()
            .any(|m| m.content == unique && m.sender == "agent"),
        "notify_user must persist to the user-facing thread"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Section 7 — reserved-thread cold-boot contract (what the session resumes from).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn scenario_reserved_threads_are_distinct_and_persist() {
    use openhuman_core::openhuman::memory_conversations::{
        append_message, ensure_thread, get_messages, ConversationMessage, CreateConversationThread,
    };

    assert_ne!(
        ORCHESTRATOR_THREAD_ID, USER_THREAD_ID,
        "orchestrator and user threads must be distinct"
    );

    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path().to_path_buf();

    // The reserved thread must exist before appending (the production code
    // ensures this lazily; mirror it here).
    ensure_thread(
        workspace.clone(),
        CreateConversationThread {
            id: ORCHESTRATOR_THREAD_ID.into(),
            title: "Subconscious Orchestrator".into(),
            created_at: "2026-06-12T00:00:00Z".into(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        },
    )
    .expect("ensure thread");

    // Simulate prior orchestrator history that a long-lived session would
    // cold-boot resume from via seed_resume_from_messages.
    for (sender, content) in [
        ("user", "[composio/gmail] new invoice"),
        ("agent", "Noted; I'll watch for the follow-up."),
    ] {
        append_message(
            workspace.clone(),
            ORCHESTRATOR_THREAD_ID,
            ConversationMessage {
                id: format!("m-{sender}"),
                content: content.into(),
                message_type: "text".into(),
                extra_metadata: serde_json::Value::Null,
                sender: sender.into(),
                created_at: "2026-06-12T00:00:00Z".into(),
            },
        )
        .expect("append");
    }

    let history = get_messages(workspace, ORCHESTRATOR_THREAD_ID).expect("read");
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].sender, "user");
    assert_eq!(history[1].sender, "agent");
}
