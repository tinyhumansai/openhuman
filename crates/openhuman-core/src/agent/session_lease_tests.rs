use super::*;

#[tokio::test]
async fn independent_runners_serialize_the_same_tenant_agent_session() {
    let key = SessionLeaseKey::new("tenant-a", "agent-a", "session-a");
    let first = SessionLease::acquire(key.clone(), None)
        .await
        .expect("first runner acquires the session");
    let cancellation = CancellationToken::new();
    let waiting = tokio::spawn({
        let key = key.clone();
        let cancellation = cancellation.clone();
        async move { SessionLease::acquire(key, Some(&cancellation)).await }
    });
    tokio::task::yield_now().await;
    assert!(
        !waiting.is_finished(),
        "same identity must wait for checkout"
    );

    drop(first);
    let second = tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .expect("released session lease wakes the next runner")
        .expect("lease task does not panic")
        .expect("second runner acquires the released session");
    drop(second);
}

#[tokio::test]
async fn distinct_tenant_agent_or_session_keys_do_not_serialize() {
    let first = SessionLease::acquire(
        SessionLeaseKey::new("tenant-a", "agent-a", "session-a"),
        None,
    )
    .await
    .expect("first lease");
    let second = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        SessionLease::acquire(
            SessionLeaseKey::new("tenant-b", "agent-a", "session-a"),
            None,
        ),
    )
    .await
    .expect("a different tenant has independent session state")
    .expect("second lease");
    drop((first, second));
}

#[tokio::test]
async fn cancelled_waiter_does_not_acquire_a_session_lease() {
    let key = SessionLeaseKey::new("tenant-a", "agent-a", "session-a");
    let first = SessionLease::acquire(key.clone(), None)
        .await
        .expect("first lease");
    let cancellation = CancellationToken::new();
    let waiting = tokio::spawn({
        let key = key.clone();
        let cancellation = cancellation.clone();
        async move { SessionLease::acquire(key, Some(&cancellation)).await }
    });
    tokio::task::yield_now().await;
    cancellation.cancel();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
            .await
            .expect("cancelled waiter completes promptly")
            .expect("lease task does not panic")
            .is_err()
    );
    drop(first);
}
