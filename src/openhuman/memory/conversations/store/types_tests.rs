//! Serde-contract tests for the conversation wire types.

use super::*;
use serde_json::json;

#[test]
fn conversation_thread_serde_uses_camel_case_and_defaults_labels() {
    let raw = json!({
        "id": "thread-1",
        "title": "Memory",
        "chatId": 42,
        "isActive": true,
        "messageCount": 3,
        "lastMessageAt": "2026-05-24T08:00:00Z",
        "createdAt": "2026-05-24T07:00:00Z",
        "parentThreadId": "parent-1"
    });

    let thread: ConversationThread = serde_json::from_value(raw).unwrap();
    assert_eq!(thread.chat_id, Some(42));
    assert_eq!(thread.parent_thread_id.as_deref(), Some("parent-1"));
    assert!(thread.labels.is_empty(), "labels should default to []");

    let encoded = serde_json::to_value(&thread).unwrap();
    assert_eq!(encoded["chatId"], json!(42));
    assert_eq!(encoded["parentThreadId"], json!("parent-1"));
    assert!(encoded.get("chat_id").is_none());
    assert!(encoded.get("parent_thread_id").is_none());
}

#[test]
fn conversation_message_patch_defaults_to_no_changes() {
    let patch: ConversationMessagePatch = serde_json::from_value(json!({})).unwrap();
    assert!(patch.extra_metadata.is_none());

    let patch_with_metadata: ConversationMessagePatch =
        serde_json::from_value(json!({"extraMetadata": {"source": "mock"}})).unwrap();
    assert_eq!(
        patch_with_metadata.extra_metadata,
        Some(json!({"source": "mock"}))
    );
}

#[test]
fn create_thread_optional_fields_roundtrip() {
    let create = CreateConversationThread {
        id: "thread-2".into(),
        title: "Thread".into(),
        created_at: "2026-05-24T08:00:00Z".into(),
        parent_thread_id: None,
        labels: Some(vec!["important".into(), "memory".into()]),
        personality_id: None,
    };

    let encoded = serde_json::to_value(&create).unwrap();
    assert_eq!(encoded["labels"], json!(["important", "memory"]));
    assert_eq!(encoded["parentThreadId"], Value::Null);

    let decoded: CreateConversationThread = serde_json::from_value(encoded).unwrap();
    assert_eq!(
        decoded.labels,
        Some(vec!["important".to_string(), "memory".to_string()])
    );
    assert!(decoded.parent_thread_id.is_none());
}
