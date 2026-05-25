//! Wire/storage types for the workspace-backed conversation store: threads,
//! messages, create requests, and partial-update patches.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A persisted conversation thread, mirroring one entry in `threads.jsonl`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationThread {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_id: Option<i64>,
    pub is_active: bool,
    pub message_count: usize,
    pub last_message_at: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
}

/// A single message appended to a thread's JSONL log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessage {
    pub id: String,
    pub content: String,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default)]
    pub extra_metadata: Value,
    pub sender: String,
    pub created_at: String,
}

/// Input payload to create-or-update a thread via [`super::ensure_thread`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateConversationThread {
    pub id: String,
    pub title: String,
    pub created_at: String,
    #[serde(default)]
    pub parent_thread_id: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
}

/// Partial update to apply to a stored message (e.g. rewriting `extraMetadata`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessagePatch {
    #[serde(default)]
    pub extra_metadata: Option<Value>,
}

/// A single match returned by
/// [`super::store::ConversationStore::search_cross_thread_messages`]. Carries
/// the source `thread_id` so the caller can render provenance into the
/// `[Cross-chat context]` block (issue #1505).
#[derive(Debug, Clone, PartialEq)]
pub struct CrossThreadHit {
    pub thread_id: String,
    pub message_id: String,
    pub role: String,
    pub content: String,
    pub created_at: String,
    pub score: f64,
}

#[cfg(test)]
mod tests {
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
}
