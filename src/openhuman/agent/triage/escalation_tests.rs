use super::*;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::harness::definition::AgentDefinitionRegistry;
use serde_json::json;
use tokio::time::{sleep, timeout, Duration};

/// The triage parent is the shared root context with one override: nested
/// spawns scoped to the dispatched target. Guards the #4369 collapse against
/// drift from `build_root_parent`.
#[tokio::test]
async fn build_triage_parent_scopes_allowed_subagents_to_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };

    let _ = AgentDefinitionRegistry::init_global_builtins();
    let ctx = build_triage_parent(&config, "researcher")
        .await
        .expect("build triage parent");

    assert_eq!(ctx.agent_definition_id, "triage");
    assert_eq!(ctx.channel, "triage");
    assert!(
        ctx.session_id.starts_with("triage-"),
        "session_id namespaced by triage prefix, got {}",
        ctx.session_id
    );
    assert_eq!(
        ctx.allowed_subagent_ids,
        ["researcher".to_string()].into_iter().collect(),
        "nested spawns must be scoped to the single dispatched target"
    );
}

static TEST_EVENTS_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct TestEventsGuard(tokio::sync::MutexGuard<'static, ()>);

impl Drop for TestEventsGuard {
    fn drop(&mut self) {
        events::clear_test_events();
    }
}

async fn test_events_guard() -> TestEventsGuard {
    let guard = TEST_EVENTS_LOCK.lock().await;
    events::clear_test_events();
    TestEventsGuard(guard)
}

fn envelope(external_id: &str) -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "triage-escalation",
        external_id,
        json!({ "subject": "hello" }),
    )
}

fn run(action: TriageAction) -> TriageRun {
    TriageRun {
        decision: super::super::decision::TriageDecision {
            action,
            target_agent: None,
            prompt: None,
            reason: "because".into(),
        },
        used_local: false,
        latency_ms: 9,
        resolution_path: super::super::evaluator::TriageResolutionPath::Cloud,
    }
}

fn run_with_target(action: TriageAction, target_agent: &str, prompt: &str) -> TriageRun {
    TriageRun {
        decision: super::super::decision::TriageDecision {
            action,
            target_agent: Some(target_agent.into()),
            prompt: Some(prompt.into()),
            reason: "because".into(),
        },
        used_local: false,
        latency_ms: 9,
        resolution_path: super::super::evaluator::TriageResolutionPath::Cloud,
    }
}

async fn collect_trigger_events_until(
    external_id: &str,
    expected: impl Fn(&[DomainEvent]) -> bool,
) -> Vec<DomainEvent> {
    let external_id = external_id.to_string();
    timeout(Duration::from_secs(5), async {
        loop {
            let captured = events::test_events_for_external_id(&external_id);
            if expected(&captured) {
                return captured;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("expected triage event should arrive")
}

#[tokio::test]
async fn apply_decision_drop_only_publishes_evaluated() {
    let _events_guard = test_events_guard().await;
    let envelope = envelope("esc-drop");
    crate::core::bus::init().await.expect("bus init");
    let collect = tokio::spawn(collect_trigger_events_until("esc-drop", |events| {
        events.iter().any(|event| {
            matches!(
                event,
                DomainEvent::TriggerEvaluated {
                    decision,
                    external_id,
                    ..
                } if decision == "drop" && external_id == "esc-drop"
            )
        })
    }));

    apply_decision(run(TriageAction::Drop), &envelope)
        .await
        .expect("drop should not fail");

    let captured = collect.await.expect("event collector should not panic");
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEvaluated {
            decision,
            external_id,
            ..
        } if decision == "drop" && external_id == "esc-drop"
    )));
    assert!(!captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalated { external_id, .. }
            | DomainEvent::TriggerEscalationFailed { external_id, .. }
            if external_id == "esc-drop"
    )));
}

#[tokio::test]
async fn apply_decision_acknowledge_only_publishes_evaluated() {
    let _events_guard = test_events_guard().await;
    let envelope = envelope("esc-ack");
    crate::core::bus::init().await.expect("bus init");
    let collect = tokio::spawn(collect_trigger_events_until("esc-ack", |events| {
        events.iter().any(|event| {
            matches!(
                event,
                DomainEvent::TriggerEvaluated {
                    decision,
                    external_id,
                    ..
                } if decision == "acknowledge" && external_id == "esc-ack"
            )
        })
    }));

    apply_decision(run(TriageAction::Acknowledge), &envelope)
        .await
        .expect("acknowledge should not fail");

    let captured = collect.await.expect("event collector should not panic");
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEvaluated {
            decision,
            external_id,
            ..
        } if decision == "acknowledge" && external_id == "esc-ack"
    )));
    assert!(!captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalated { external_id, .. }
            | DomainEvent::TriggerEscalationFailed { external_id, .. }
            if external_id == "esc-ack"
    )));
}

async fn seed_task_card() -> (
    tempfile::TempDir,
    crate::openhuman::threads::todos::ops::BoardLocation,
    String,
) {
    use crate::openhuman::threads::todos::ops::{self, BoardLocation, CardPatch};
    let dir = tempfile::tempdir().unwrap();
    let location = BoardLocation::Thread {
        workspace_dir: dir.path().to_path_buf(),
        thread_id: "task-sources".to_string(),
    };
    let card_id = ops::add(&location, "ingested issue", CardPatch::default())
        .await
        .unwrap()
        .cards[0]
        .id
        .clone();
    (dir, location, card_id)
}

#[tokio::test]
async fn apply_decision_drop_gates_linked_card_to_rejected() {
    use crate::openhuman::agent::task_board::TaskCardStatus;
    use crate::openhuman::threads::todos::ops;

    let _events_guard = test_events_guard().await;
    crate::core::bus::init().await.expect("bus init");
    let (_dir, location, card_id) = seed_task_card().await;

    let envelope = envelope("esc-drop-card").with_task_card(card_id.clone(), location.clone());
    apply_decision(run(TriageAction::Drop), &envelope)
        .await
        .expect("drop should not fail");

    let status = ops::list(&location)
        .await
        .unwrap()
        .cards
        .into_iter()
        .find(|c| c.id == card_id)
        .map(|c| c.status);
    assert_eq!(
        status,
        Some(TaskCardStatus::Rejected),
        "a dropped card-linked trigger must be gated terminally so the board poller skips it"
    );
}

#[tokio::test]
async fn apply_decision_acknowledge_gates_linked_card_to_rejected() {
    use crate::openhuman::agent::task_board::TaskCardStatus;
    use crate::openhuman::threads::todos::ops;

    let _events_guard = test_events_guard().await;
    crate::core::bus::init().await.expect("bus init");
    let (_dir, location, card_id) = seed_task_card().await;

    let envelope = envelope("esc-ack-card").with_task_card(card_id.clone(), location.clone());
    apply_decision(run(TriageAction::Acknowledge), &envelope)
        .await
        .expect("acknowledge should not fail");

    let status = ops::list(&location)
        .await
        .unwrap()
        .cards
        .into_iter()
        .find(|c| c.id == card_id)
        .map(|c| c.status);
    assert_eq!(status, Some(TaskCardStatus::Rejected));
}

#[tokio::test]
async fn apply_decision_react_failure_publishes_failed_event() {
    let _events_guard = test_events_guard().await;
    let envelope = envelope("esc-react-fail");
    crate::core::bus::init().await.expect("bus init");
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let missing_target = format!("missing-agent-{}", uuid::Uuid::new_v4());
    let collect = tokio::spawn(collect_trigger_events_until("esc-react-fail", |events| {
        events.iter().any(|event| {
            matches!(
                event,
                DomainEvent::TriggerEvaluated {
                    decision,
                    external_id,
                    ..
                } if decision == "react" && external_id == "esc-react-fail"
            )
        }) && events.iter().any(|event| {
            matches!(
                event,
                DomainEvent::TriggerEscalationFailed { external_id, .. }
                    if external_id == "esc-react-fail"
            )
        })
    }));

    let result = apply_decision(
        run_with_target(TriageAction::React, &missing_target, "handle this"),
        &envelope,
    )
    .await;
    if let Err(err) = result {
        assert!(err.to_string().contains(&missing_target));
    }

    let captured = collect.await.expect("event collector should not panic");
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEvaluated {
            decision,
            external_id,
            ..
        } if decision == "react" && external_id == "esc-react-fail"
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalationFailed { external_id, .. }
            if external_id == "esc-react-fail"
    )));
}

#[tokio::test]
async fn apply_decision_escalate_failure_publishes_failed_event() {
    let _events_guard = test_events_guard().await;
    let envelope = envelope("esc-escalate-fail");
    crate::core::bus::init().await.expect("bus init");
    let _ = AgentDefinitionRegistry::init_global_builtins();
    let missing_target = format!("missing-agent-{}", uuid::Uuid::new_v4());
    let collect = tokio::spawn(collect_trigger_events_until(
        "esc-escalate-fail",
        |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::TriggerEvaluated {
                        decision,
                        external_id,
                        ..
                    } if decision == "escalate" && external_id == "esc-escalate-fail"
                )
            }) && events.iter().any(|event| {
                matches!(
                    event,
                    DomainEvent::TriggerEscalationFailed { external_id, .. }
                        if external_id == "esc-escalate-fail"
                )
            })
        },
    ));

    let result = apply_decision(
        run_with_target(TriageAction::Escalate, &missing_target, "escalate this"),
        &envelope,
    )
    .await;
    if let Err(err) = result {
        assert!(err.to_string().contains(&missing_target));
    }

    let captured = collect.await.expect("event collector should not panic");
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEvaluated {
            decision,
            external_id,
            ..
        } if decision == "escalate" && external_id == "esc-escalate-fail"
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalationFailed { external_id, .. }
            if external_id == "esc-escalate-fail"
    )));
}

/// Only the composio path archives its input before triage. Naming a
/// trigger-history record for a webhook or cron acknowledge would send an
/// operator looking for a file that was never written.
#[test]
fn only_composio_claims_a_retained_input() {
    assert_eq!(
        retained_input_note(&TriggerSource::Composio {
            toolkit: "gmail".into(),
            trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
        }),
        "trigger-history archive"
    );

    for source in [
        TriggerSource::Webhook {
            tunnel_id: "t".into(),
            method: "POST".into(),
            path: "/x".into(),
        },
        TriggerSource::Cron {
            job_id: "j".into(),
            job_name: "nightly".into(),
        },
        TriggerSource::WebviewIntegration {
            provider: "gmail".into(),
            account_id: "a".into(),
        },
    ] {
        assert_eq!(
            retained_input_note(&source),
            "none — verdict only",
            "{} has no archive to point at",
            source.slug()
        );
    }
}
