use super::*;
use serde_json::json;

#[test]
fn core_notification_backward_compat_no_actions() {
    let json = json!({
        "id": "test-1",
        "category": "system",
        "title": "Hello",
        "body": "World",
        "timestamp_ms": 123456
    });
    let event: CoreNotificationEvent = serde_json::from_value(json).unwrap();
    assert!(event.actions.is_none());
    assert!(event.deep_link.is_none());
}

#[test]
fn core_notification_with_actions() {
    let json = json!({
        "id": "test-2",
        "category": "meetings",
        "title": "Join call?",
        "body": "Standup in 5 min",
        "timestamp_ms": 999,
        "actions": [
            {"actionId": "yes", "label": "Yes"},
            {"actionId": "no", "label": "No", "payload": {"meeting_id": "m1"}}
        ]
    });
    let event: CoreNotificationEvent = serde_json::from_value(json).unwrap();
    let actions = event.actions.unwrap();
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].action_id, "yes");
    assert!(actions[0].payload.is_none());
    assert_eq!(actions[1].action_id, "no");
    assert!(actions[1].payload.is_some());
}

#[test]
fn core_notification_serialize_skips_empty_actions() {
    let event = CoreNotificationEvent {
        id: "x".into(),
        category: CoreNotificationCategory::System,
        title: "t".into(),
        body: "b".into(),
        deep_link: None,
        timestamp_ms: 1,
        actions: None,
        workspace: None,
        workspace_revision: None,
    };
    let s = serde_json::to_string(&event).unwrap();
    assert!(!s.contains("actions"));
}
