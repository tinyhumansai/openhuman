use super::*;
use tinybus::{Message, MessageKind};

fn signal(member: &str, body: serde_json::Value) -> Message {
    Message::signal(
        names::OBJECT_PATH.try_into().unwrap(),
        names::INTERFACE.try_into().unwrap(),
        member.try_into().unwrap(),
        body,
    )
}

fn from(mut message: Message, sender: &str) -> Message {
    message.header.sender = Some(sender.try_into().unwrap());
    message
}

fn event_body() -> serde_json::Value {
    serde_json::to_value((PolicyChanged {
        generation: 7,
        policy_id: None,
        policy_generation: None,
    },))
    .unwrap()
}

#[test]
fn a_policy_changed_signal_from_the_module_is_decoded() {
    let message = from(signal(names::POLICY_CHANGED, event_body()), ":1.9");
    let event = decode_policy_changed(&message, ":1.9").expect("decoded");
    assert_eq!(event.generation, 7);
}

#[test]
fn a_signal_from_another_sender_is_ignored() {
    let message = from(signal(names::POLICY_CHANGED, event_body()), ":1.10");
    assert!(decode_policy_changed(&message, ":1.9").is_none());
}

#[test]
fn an_unsigned_signal_is_ignored() {
    let message = signal(names::POLICY_CHANGED, event_body());
    assert!(decode_policy_changed(&message, ":1.9").is_none());
}

#[test]
fn another_member_or_a_malformed_body_is_ignored() {
    let other = from(signal("SomethingElse", event_body()), ":1.9");
    assert!(decode_policy_changed(&other, ":1.9").is_none());
    let bad = from(
        signal(names::POLICY_CHANGED, serde_json::json!({"nope": 1})),
        ":1.9",
    );
    assert!(decode_policy_changed(&bad, ":1.9").is_none());
}

#[test]
fn a_method_call_with_the_same_member_is_not_a_signal() {
    let mut message = from(signal(names::POLICY_CHANGED, event_body()), ":1.9");
    message.header.kind = MessageKind::MethodCall;
    assert!(decode_policy_changed(&message, ":1.9").is_none());
}
