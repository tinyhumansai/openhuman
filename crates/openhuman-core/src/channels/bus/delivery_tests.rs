use super::*;
use serde_json::json;

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
            turn_id: None,
            channel_ref: None,
            conversation: None,
            idempotency_key: None,
            message: json!({"text":"hello"}),
        },
        ProgressiveTransportRequest::SendTyped {
            channel: "discord:fixture".into(),
            turn_id: None,
            channel_ref: None,
            conversation: None,
            sequence: None,
            segment_index: None,
            idempotency_key: None,
            parts: Vec::new(),
            text: "hello".into(),
        },
        ProgressiveTransportRequest::Edit {
            channel: "discord:fixture".into(),
            turn_id: None,
            channel_ref: None,
            conversation: None,
            receipt: None,
            message_id: "message".into(),
            text: "edited".into(),
        },
        ProgressiveTransportRequest::Delete {
            channel: "discord:fixture".into(),
            channel_ref: None,
            conversation: None,
            receipt: None,
            message_id: "message".into(),
        },
        ProgressiveTransportRequest::Typing {
            channel: "discord:fixture".into(),
            channel_ref: None,
            conversation: None,
        },
        ProgressiveTransportRequest::OutputEvent {
            channel: "discord:fixture".into(),
            channel_ref: None,
            conversation: None,
            turn_id: "turn".into(),
            sequence: 1,
            event: Box::new(ChannelOutputEvent::TextDelta {
                text: "hello".into(),
                iteration: None,
            }),
        },
    ] {
        assert_eq!(request_channel(&request), "discord:fixture");
    }
}
