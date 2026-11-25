use super::*;
use tinychannels_bus::web::{WebChannelEvent as Event, WebEventContext};

fn context() -> WebEventContext {
    WebEventContext {
        client_id: "client".into(),
        thread_id: "thread".into(),
        request_id: "request".into(),
        seq: Some(4),
        ts: Some(99),
        ..Default::default()
    }
}

#[test]
fn scoped_event_serializes_the_owner_wire_shape_without_host_routing_stamps() {
    let event = ScopedWebEvent::new(Event::ChatDone {
        context: context(),
        full_response: Some("hello".into()),
        citations: None,
        usage: None,
        timing: None,
        parts: None,
    });
    let wire = serde_json::to_value(&event).expect("event serializes");
    assert_eq!(wire["event"], "chat_done");
    assert_eq!(wire["client_id"], "client");
    assert_eq!(wire["thread_id"], "thread");
    assert_eq!(wire["request_id"], "request");
    assert_eq!(wire["seq"], 4);
    assert_eq!(wire["ts"], 99);
    assert_eq!(wire["full_response"], "hello");
    assert!(wire.get("agent").is_none());
    assert!(wire.get("profile").is_none());
    assert_eq!(event.event_name(), "chat_done");
}

#[test]
fn host_tenant_stamps_route_without_changing_the_typed_event_payload() {
    let mut event = ScopedWebEvent::new(Event::ChatCancelled {
        context: context(),
        cancel_reason: Some("user".into()),
        superseded_by: None,
    });
    event.stamp_tenant(Some(crate::core::runtime::Tenant {
        profile: Some("profile".into()),
        agent: Some("agent".into()),
    }));

    assert!(event.belongs_to("agent"));
    assert!(event.belongs_to_profile("profile"));
    assert!(!event.belongs_to("another-agent"));
    let wire = serde_json::to_value(&event).expect("event serializes");
    assert_eq!(wire["event"], "chat_cancelled");
    assert_eq!(wire["cancel_reason"], "user");
    assert!(wire.get("agent").is_none());
    assert!(wire.get("profile").is_none());
}
