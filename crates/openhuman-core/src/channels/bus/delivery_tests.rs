use super::*;
use serde_json::json;

#[test]
fn outbound_body_has_stable_idempotency_key() {
    let body = json!({ "text": "hello" });
    let first = channel_message_body_with_idempotency("telegram", body.clone());
    let second = channel_message_body_with_idempotency("telegram", body);

    assert_eq!(first, second);
    assert!(first.get("idempotencyKey").is_some());
}

#[tokio::test]
async fn opening_without_a_registered_callback_answers_the_unavailable_sentinel() {
    let error = ModuleProgressiveReply::open(
        crate::config::Config::default(),
        "telegram".into(),
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .err()
    .expect("no callback registered");
    assert!(
        error
            .to_string()
            .starts_with(crate::core::observability::BACKEND_UNAVAILABLE_PREFIX),
        "{error}"
    );
}

#[test]
fn every_transport_operation_carries_the_channel_scoped_by_the_host() {
    for request in [
        ProgressiveTransportRequest::Send {
            channel: "discord:fixture".into(),
            message: json!({"text":"hello"}),
        },
        ProgressiveTransportRequest::Edit {
            channel: "discord:fixture".into(),
            message_id: "message".into(),
            text: "edited".into(),
        },
        ProgressiveTransportRequest::Delete {
            channel: "discord:fixture".into(),
            message_id: "message".into(),
        },
        ProgressiveTransportRequest::Typing {
            channel: "discord:fixture".into(),
        },
    ] {
        assert_eq!(request_channel(&request), "discord:fixture");
    }
}
