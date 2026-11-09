//! Tests for a trigger envelope as an inbound event, and back.

use serde_json::json;
use tinyeventqueues_bus::Lane;

use super::*;

fn composio_envelope(uuid: &str, id: &str) -> TriggerEnvelope {
    TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        id,
        uuid,
        json!({ "subject": "hi" }),
    )
}

#[test]
fn a_composio_trigger_round_trips_with_a_stable_identity() {
    let envelope = composio_envelope("uuid-1", "id-1");
    let event = composio(&envelope).unwrap();
    assert_eq!(event.source, "composio:gmail");
    assert_eq!(event.event_id, "uuid-1");
    assert_eq!(event.conversation_key, "composio:gmail:uuid-1");
    assert_eq!(event.kind, EventKind::Trigger);
    assert!(event
        .text
        .starts_with("composio/gmail/GMAIL_NEW_GMAIL_MESSAGE"));
    // A redelivery is the same event to the queues.
    assert_eq!(
        composio(&composio_envelope("uuid-1", "id-1"))
            .unwrap()
            .dedupe_key(),
        event.dedupe_key()
    );

    let back = envelope_of(&event).unwrap();
    assert_eq!(back.external_id, envelope.external_id);
    assert_eq!(back.display_label, envelope.display_label);
    assert_eq!(back.payload, envelope.payload);
    assert_eq!(
        back.received_at.timestamp_millis(),
        envelope.received_at.timestamp_millis()
    );
    assert!(matches!(
        back.source,
        TriggerSource::Composio { ref toolkit, ref trigger }
            if toolkit == "gmail" && trigger == "GMAIL_NEW_GMAIL_MESSAGE"
    ));
}

#[test]
fn a_composio_trigger_without_a_uuid_uses_its_id_or_a_fresh_one() {
    assert_eq!(
        composio(&composio_envelope("", "id-7")).unwrap().event_id,
        "id-7"
    );
    let fresh = composio(&composio_envelope("", "")).unwrap();
    assert!(uuid::Uuid::parse_str(&fresh.event_id).is_ok());
}

#[test]
fn a_webhook_is_identified_by_its_correlation_id() {
    let envelope = TriggerEnvelope::from_webhook("tunnel-1", "POST", "hooks/x", json!({"a": 1}));
    let event = webhook(&envelope, "corr-1").unwrap();
    assert_eq!(event.source, "webhook:tunnel-1");
    assert_eq!(event.event_id, "corr-1");
    let back = envelope_of(&event).unwrap();
    assert!(matches!(
        back.source,
        TriggerSource::Webhook { ref tunnel_id, ref method, ref path }
            if tunnel_id == "tunnel-1" && method == "POST" && path == "hooks/x"
    ));
    assert!(!webhook(&envelope, " ").unwrap().event_id.trim().is_empty());
}

#[test]
fn only_matching_sources_are_queued() {
    let cron = TriggerEnvelope::from_cron("job", "nightly", "ok");
    assert_eq!(composio(&cron).unwrap_err(), Unqueueable::Source);
    assert_eq!(webhook(&cron, "c").unwrap_err(), Unqueueable::Source);
    assert_eq!(Unqueueable::Source.to_string(), "source not queued");
}

#[test]
fn an_oversize_envelope_takes_the_direct_path() {
    let big = "x".repeat(MAX_RAW_BYTES);
    let envelope = TriggerEnvelope::from_composio("gmail", "T", "", "u", json!({ "body": big }));
    let refused = composio(&envelope).unwrap_err();
    assert!(matches!(refused, Unqueueable::Oversize(bytes) if bytes > MAX_RAW_BYTES));
    assert!(refused.to_string().contains("exceeds the queue limit"));
}

#[test]
fn an_event_that_is_not_a_trigger_has_no_envelope() {
    let mut event = composio(&composio_envelope("u", "")).unwrap();
    event.raw = None;
    assert!(envelope_of(&event).is_none());
    event.raw = Some(json!({
        "kind": "carrier-pigeon",
        "external_id": "x",
        "display_label": "x",
        "payload": {},
        "received_at_ms": 0,
    }));
    assert!(envelope_of(&event).is_none());
    event.raw = Some(json!({ "kind": "composio" }));
    assert!(envelope_of(&event).is_none());
}

#[test]
fn a_digest_lists_every_folded_event() {
    let trigger = composio(&composio_envelope("u1", "")).unwrap();
    let chat = InboundEvent::new(
        "web:chat",
        "9",
        "c",
        Sender::new("x"),
        EventKind::Message,
        0,
    )
    .text("hello");
    let turn = Turn::new("t-9", Lane::Digest, "c", vec![trigger, chat]);
    let digest = digest_envelope(&turn);
    assert_eq!(digest.display_label, "external/inbound-digest/t-9");
    assert_eq!(digest.payload["count"], 2);
    assert_eq!(digest.payload["events"][0]["event_id"], "u1");
    assert_eq!(
        digest.payload["events"][0]["label"],
        "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE"
    );
    assert_eq!(digest.payload["events"][1]["text"], "hello");
}

#[test]
fn an_external_event_is_keyed_by_its_source_and_key_and_round_trips() {
    let payload = json!({ "author": "@alice", "conversation_id": "c1" });
    let event = external(
        "x:mentions",
        "mentions",
        "t1",
        "hi @bot",
        payload.clone(),
        42,
    )
    .unwrap();
    assert_eq!(event.source, "x:mentions");
    assert_eq!(event.event_id, "t1");
    assert_eq!(event.conversation_key, "mentions");
    assert_eq!(event.sender.id, "x:mentions");
    assert_eq!(event.kind, EventKind::Trigger);
    assert_eq!(event.received_at_ms, 42);
    assert_eq!(event.text, "hi @bot");
    assert!(envelope_of(&event).is_none(), "not a trigger envelope");
    let item = external_of(&event).unwrap();
    assert_eq!(
        serde_json::to_value(&item).unwrap(),
        json!({ "id": "t1", "text": "hi @bot", "payload": payload, "received_at_ms": 42 })
    );
    let trigger = composio(&composio_envelope("u", "i")).unwrap();
    assert!(external_of(&trigger).is_none());
}

#[test]
fn an_oversize_external_event_is_refused() {
    let big = "x".repeat(MAX_RAW_BYTES);
    let refused = external(
        "x:mentions",
        "mentions",
        "t1",
        "",
        json!({ "blob": big }),
        0,
    );
    assert!(matches!(refused, Err(Unqueueable::Oversize(_))));
}
