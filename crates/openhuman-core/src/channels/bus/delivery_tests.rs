use super::*;
use crate::config::test_env::EnvVarGuard;

#[test]
fn outbound_body_has_stable_idempotency_key() {
    let body = json!({ "text": "hello" });
    let first = channel_message_body_with_idempotency("telegram", body.clone());
    let second = channel_message_body_with_idempotency("telegram", body);

    assert_eq!(first, second);
    assert!(first.get("idempotencyKey").is_some());
}

#[tokio::test]
async fn transport_captures_no_local_or_missing_hosted_credential() {
    let _env_lock = crate::config::TEST_ENV_LOCK.lock().await;
    let workspace = tempfile::tempdir().expect("temporary workspace");
    let _workspace_env = EnvVarGuard::workspace_unlocked(workspace.path());
    assert!(ManagedTransport::capture(
        &crate::config::Config::default(),
        "telegram".into(),
        tokio_util::sync::CancellationToken::new()
    )
    .is_err());
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

#[test]
fn a_cancelled_identity_cannot_capture_a_replacement_credential() {
    let session = tokio_util::sync::CancellationToken::new();
    session.cancel();
    let error = ManagedTransport::capture(
        &crate::config::Config::default(),
        "telegram".into(),
        session,
    )
    .err()
    .expect("closed identities must fail before credential reads");
    assert_eq!(error.to_string(), "Channel session closed");
}
