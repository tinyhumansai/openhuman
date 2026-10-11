//! Completed dispatches retain their outcome when control arrives afterward.
use super::*;

#[tokio::test]
async fn completed_replies_survive_later_cancellation() {
    let native = crate::CancellationToken::new();
    let token = crate::CancellationToken::new();
    let handle = crate::TurnCancellation::default();
    let reply = Ok("completed reply");
    token.cancel();
    native.cancel();
    handle.cancel().await;
    assert_eq!(
        controlled_outcome(reply, &native, Some(&token), &handle, None).unwrap(),
        "completed reply"
    );
}

#[test]
fn completed_failures_survive_later_deadlines() {
    let native = crate::CancellationToken::new();
    native.cancel();
    let outcome = controlled_outcome::<()>(
        Err(CoreError::InvalidRoute { method: AGENT_CHAT }),
        &native,
        None,
        &crate::TurnCancellation::default(),
        Some(tokio::time::Instant::now()),
    );
    assert!(matches!(outcome, Err(CoreError::InvalidRoute { .. })));
}

#[test]
fn a_native_cancelled_dispatch_keeps_the_deadline_reason() {
    let native = crate::CancellationToken::new();
    native.cancel();
    let outcome = controlled_outcome::<()>(
        Err(CoreError::Rpc {
            method: AGENT_CHAT,
            message: "session turn cancelled".into(),
        }),
        &native,
        None,
        &crate::TurnCancellation::default(),
        Some(tokio::time::Instant::now()),
    );
    assert!(matches!(outcome, Err(CoreError::DeadlineExceeded { .. })));
}
