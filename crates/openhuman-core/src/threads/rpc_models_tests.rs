use super::*;

#[test]
fn queued_chat_ack_serializes_request_identity_and_opaque_refs() {
    let ack = QueuedChatAck {
        queued: true,
        queue_mode: "followup".into(),
        client_id: "client".into(),
        thread_id: "thread".into(),
        request_id: "request".into(),
        queue_depth: 1,
        queued_attachments: vec![],
    };
    let value = serde_json::to_value(ack).unwrap();
    assert_eq!(value["request_id"], "request");
    assert_eq!(value["queued_attachments"], serde_json::json!([]));
    assert!(value.get("requestId").is_none());
}
