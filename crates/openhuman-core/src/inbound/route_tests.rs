//! Tests for the hand-off to the queues.

use std::time::Duration;

use serde_json::json;

use super::*;
use crate::inbound::events;
use crate::inbound::fixture::{active_config, Rig, ScriptedTriage, Step};
use crate::triage::TriggerEnvelope;

fn event(uuid: &str) -> InboundEvent {
    let envelope = TriggerEnvelope::from_composio("slack", "SLACK_MESSAGE", "", uuid, json!({}));
    events::composio(&envelope).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_accepted_event_is_queued_and_a_redelivery_ends_at_ingress() {
    let rig = Rig::new(ScriptedTriage::new(Vec::<Step>::new())).await;
    let config = active_config();
    let first = offer(&config, &rig.client, event("u1")).await;
    assert_eq!(first, Offered::Queued(Disposition::Accepted));
    assert!(first.is_queued());
    let again = offer(&config, &rig.client, event("u1")).await;
    assert_eq!(again, Offered::Queued(Disposition::Duplicate));
    let _ = rig.triage.until_applied(1, Duration::from_secs(5)).await;
}

#[tokio::test]
async fn inactive_queues_send_the_event_down_the_direct_path() {
    let mut config = active_config();
    config.inbound.enabled = false;
    let client = crate::modules::ops::client(config.clone());
    let offered = offer(&config, &client, event("u2")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("inactive")));
    assert!(!offered.is_queued());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unreachable_module_sends_the_event_down_the_direct_path() {
    // A client bound to an object nobody serves: the call fails.
    let bus = tinybus::transport::memory::MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let caller = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = caller
        .proxy(
            tinyeventqueues_bus::INTERFACE,
            tinyeventqueues_bus::OBJECT_PATH,
            tinyeventqueues_bus::INTERFACE,
        )
        .unwrap();
    let client = crate::modules::ops::fixture_client(proxy);
    let offered = offer(&active_config(), &client, event("u3")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
}

#[test]
fn inbound_events_belong_to_the_inbound_domain_and_name_themselves() {
    use crate::core::events::DomainEvent;
    for (event, name) in [
        (
            DomainEvent::InboundEventDropped {
                source: "composio:gmail".into(),
                event_id: "e1".into(),
                reason: "duplicate".into(),
            },
            "InboundEventDropped",
        ),
        (
            DomainEvent::InboundDeferred {
                source: "composio:gmail".into(),
                event_id: "e1".into(),
                defer_until_ms: 1_000,
                reason: "busy".into(),
            },
            "InboundDeferred",
        ),
        (
            DomainEvent::InboundDeadLettered {
                source: "webhook:t1".into(),
                event_id: "c1".into(),
                attempt: 5,
                detail: "terminal".into(),
            },
            "InboundDeadLettered",
        ),
    ] {
        assert_eq!(event.domain(), "inbound");
        assert_eq!(event.variant_name(), name);
    }
}

#[tokio::test]
async fn a_module_that_never_saw_the_event_is_not_sent_it_again() {
    // Modules off for the client: every call is refused as unavailable, which
    // proves the event never reached the queues, so there is nothing to
    // re-send and the direct path runs at once.
    let mut off = active_config();
    off.modules.enabled = false;
    let client = crate::modules::ops::client(off);
    let started = std::time::Instant::now();
    let offered = offer(&active_config(), &client, event("u4")).await;
    assert!(matches!(offered, Offered::Direct(ref why) if why.contains("ingest failed")));
    assert!(
        started.elapsed() < RESEND_PAUSE,
        "re-sent an event the module never saw: {:?}",
        started.elapsed()
    );
}
