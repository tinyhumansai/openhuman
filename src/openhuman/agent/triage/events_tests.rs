use super::*;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::triage::TriggerEnvelope;
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn publish_helpers_emit_expected_trigger_events() {
    crate::core::bus::init().await.expect("bus init");
    let seen = Arc::new(Mutex::new(Vec::<DomainEvent>::new()));
    let seen_handler = Arc::clone(&seen);
    let _handle = crate::core::bus::BUS
        .get()
        .unwrap()
        .on("triage-events-test", move |event| {
            let seen = Arc::clone(&seen_handler);
            let cloned = event.clone();
            Box::pin(async move {
                seen.lock().await.push(cloned);
            })
        });

    let envelope = TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "trig-events",
        "evt-123",
        json!({ "subject": "Coverage" }),
    );

    publish_evaluated(&envelope, "acknowledge", true, 42);
    publish_escalated(&envelope, "trigger_reactor");
    publish_failed(&envelope, "boom");

    sleep(Duration::from_millis(20)).await;

    let captured = seen.lock().await;
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEvaluated {
            source,
            external_id,
            decision,
            used_local,
            latency_ms,
            ..
        } if source == "composio"
            && external_id == "evt-123"
            && decision == "acknowledge"
            && *used_local
            && *latency_ms == 42
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalated {
            external_id,
            target_agent,
            ..
        } if external_id == "evt-123" && target_agent == "trigger_reactor"
    )));
    assert!(captured.iter().any(|event| matches!(
        event,
        DomainEvent::TriggerEscalationFailed {
            external_id,
            reason,
            ..
        } if external_id == "evt-123" && reason == "boom"
    )));
}
