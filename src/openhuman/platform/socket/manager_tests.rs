use super::*;
use serde_json::json;

#[test]
fn new_manager_is_disconnected_with_no_sid() {
    let mgr = SocketManager::new();
    let state = mgr.get_state();
    assert_eq!(state.status, ConnectionStatus::Disconnected);
    assert!(state.socket_id.is_none());
    assert!(state.error.is_none());
    assert!(!mgr.is_connected());
}

#[test]
fn default_impl_matches_new() {
    let a = SocketManager::new();
    let b = SocketManager::default();
    assert_eq!(a.get_state().status, b.get_state().status);
}

#[test]
fn is_connected_tracks_status_transitions() {
    let mgr = SocketManager::new();
    assert!(!mgr.is_connected());
    *mgr.shared.status.write() = ConnectionStatus::Connected;
    assert!(mgr.is_connected());
    *mgr.shared.status.write() = ConnectionStatus::Error;
    assert!(!mgr.is_connected());
}

#[test]
fn get_state_reflects_stored_sid_and_status() {
    let mgr = SocketManager::new();
    *mgr.shared.status.write() = ConnectionStatus::Connected;
    *mgr.shared.socket_id.write() = Some("sid-abc".to_string());
    let state = mgr.get_state();
    assert_eq!(state.status, ConnectionStatus::Connected);
    assert_eq!(state.socket_id.as_deref(), Some("sid-abc"));
}

#[test]
fn get_state_surfaces_stored_error_to_callers() {
    let mgr = SocketManager::new();
    *mgr.shared.error.write() = Some("backend redirected ws→wss; update BACKEND_URL".to_string());
    let state = mgr.get_state();
    assert_eq!(
        state.error.as_deref(),
        Some("backend redirected ws→wss; update BACKEND_URL")
    );
}

#[tokio::test]
async fn emit_without_connection_errors_without_panic() {
    let mgr = SocketManager::new();
    let err = mgr.emit("test.event", json!({"k":"v"})).await.unwrap_err();
    assert_eq!(err, "Not connected");
}

#[tokio::test]
async fn emit_while_connecting_errors_even_with_emit_channel_present() {
    // Arrange: mirror `spawn_loop`'s pre-handshake state — the emit channel is
    // installed (so the old channel-only guard would pass) but the connection's
    // readiness flag is still `false` because the SIO handshake has not
    // completed.
    let mgr = SocketManager::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx,
        ready: Arc::new(Mutex::new(false)),
    });
    *mgr.shared.status.write() = ConnectionStatus::Connecting;

    // Act
    let err = mgr
        .emit("test.event", json!({ "k": "v" }))
        .await
        .unwrap_err();

    // Assert: the caller is told the truth, and nothing was queued onto a
    // channel whose message `drain_pending_emits` would discard (#4355).
    assert_eq!(err, "Not connected");
    assert!(
        rx.try_recv().is_err(),
        "emit must not queue a message before the connection is ready"
    );
}

#[tokio::test]
async fn emit_when_connected_queues_the_encoded_event() {
    // Arrange: a handshaked connection — its readiness flag is set, mirroring
    // what `run_connection` does after the SIO CONNECT ACK.
    let mgr = SocketManager::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx,
        ready: Arc::new(Mutex::new(true)),
    });
    *mgr.shared.status.write() = ConnectionStatus::Connected;

    // Act
    mgr.emit("test.event", json!({ "k": "v" }))
        .await
        .expect("emit must succeed once the connection is ready");

    // Assert: the encoded Socket.IO event lands on the outbound channel.
    let queued = rx
        .try_recv()
        .expect("expected the emitted event to be queued");
    assert_eq!(queued, r#"42["test.event",{"k":"v"}]"#);
}

/// F2 (TOCTOU with reconnect): a reconnect that swaps in a fresh, not-yet-ready
/// channel while an `emit` is in flight must not let that `emit` succeed onto
/// the new pre-handshake channel. The readiness flag travels with the channel,
/// so replacing the channel replaces its flag too — a `status`-only gate would
/// have enqueued onto the new channel and returned `Ok`, then `drain_pending_emits`
/// would have discarded it (the exact false success #6084 targets).
#[tokio::test]
async fn emit_does_not_land_on_a_reconnects_pre_handshake_channel() {
    let mgr = SocketManager::new();

    // Old, live connection: ready channel + Connected status.
    let (_old_tx, mut old_rx) = mpsc::unbounded_channel::<String>();
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx: _old_tx,
        ready: Arc::new(Mutex::new(true)),
    });
    *mgr.shared.status.write() = ConnectionStatus::Connected;

    // A reconnect begins: `spawn_loop` would flip status→Connecting and install
    // a fresh channel whose readiness flag is still `false` (handshake pending).
    // We reproduce exactly that swap.
    let (new_tx, mut new_rx) = mpsc::unbounded_channel::<String>();
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx: new_tx,
        ready: Arc::new(Mutex::new(false)),
    });
    // Status may still read Connected in the narrow window before the loop
    // updates it — leaving it Connected here is the adversarial case a
    // `status`-only gate would have failed.
    *mgr.shared.status.write() = ConnectionStatus::Connected;

    // Act
    let err = mgr
        .emit("test.event", json!({ "k": "v" }))
        .await
        .unwrap_err();

    // Assert: rejected, and nothing queued onto either channel.
    assert_eq!(err, "Not connected");
    assert!(
        new_rx.try_recv().is_err(),
        "emit must not enqueue onto a reconnect's pre-handshake channel"
    );
    assert!(
        old_rx.try_recv().is_err(),
        "emit must not enqueue onto the replaced channel either"
    );
}

/// F1 (server `error` on a still-live socket): a server-emitted `error` event
/// flips the presentation status to `Error` but does not tear down the
/// transport, so the connection is still live and its readiness flag is
/// untouched. `emit` must therefore keep succeeding — a gate that rejected on
/// `status != Connected` would wedge every subsequent emit forever.
#[tokio::test]
async fn emit_still_succeeds_after_server_error_on_live_connection() {
    let mgr = SocketManager::new();

    // Live, handshaked connection.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx,
        ready: Arc::new(Mutex::new(true)),
    });
    *mgr.shared.status.write() = ConnectionStatus::Connected;

    // Drive the real handler for a server `error` event. It sets status=Error
    // on the same live socket without closing the transport — and has no access
    // to the readiness flag, which is owned by the connection loop.
    let (sink_tx, _sink_rx) = mpsc::unbounded_channel::<String>();
    super::super::event_handlers::handle_sio_event(
        "error",
        json!({ "message": "boom" }),
        &sink_tx,
        &mgr.shared,
    );
    assert_eq!(
        *mgr.shared.status.read(),
        ConnectionStatus::Error,
        "server error event must flip presentation status to Error"
    );

    // Act: the socket is still live, so emit must still be delivered.
    mgr.emit("test.event", json!({ "k": "v" }))
        .await
        .expect("emit must still succeed on a live socket after a server error");

    let queued = rx
        .try_recv()
        .expect("the event must be queued for the live connection");
    assert_eq!(queued, r#"42["test.event",{"k":"v"}]"#);
}

/// Teardown-vs-emit interleaving (the CodeRabbit Major on #6103): the
/// background loop clears the readiness flag and drains the emit queue on
/// teardown, and `emit` checks the flag then sends. If those two are not
/// mutually exclusive, an `emit` that observed `ready == true` can have its
/// `tx.send` land *after* the drain — leaving a stale message in the channel
/// that the next reconnect (a fresh sid whose roster the backend cleared)
/// forwards. This is the exact race `medulla::workflows::with_live_connection`
/// already guards for the medulla handlers; the bare `emit` path is closed by
/// making both critical sections take the connection's `ready` lock.
///
/// The barrier: the test task takes the `ready` lock (standing in for teardown
/// holding the gate) *before* clearing it, then spawns an `emit`. If `emit` and
/// teardown share the lock, `emit` blocks at its readiness check and cannot
/// complete while the guard is held — proven by the pending-while-locked
/// assertion. Teardown then runs under the held lock (clear + drain) exactly as
/// `ws_loop` does, releases, and only then does `emit` resume — observing
/// `ready == false` and enqueuing nothing. Reverting the flag to a bare
/// `AtomicBool` (no shared lock) removes the block, so `emit` would slip its
/// send past the drain and this test's guarantees would no longer hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn emit_cannot_send_past_a_concurrent_teardown_drain() {
    let mgr = Arc::new(SocketManager::new());

    // A live, handshaked connection. Keep a clone of its readiness gate and its
    // receiver so the test can drive teardown's clear+drain directly.
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    let gate = Arc::new(Mutex::new(true));
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx,
        ready: Arc::clone(&gate),
    });
    *mgr.shared.status.write() = ConnectionStatus::Connected;

    // Teardown begins: take the gate before clearing it, mirroring `ws_loop`'s
    // "clear the flag and drain under the same lock" critical section.
    let mut teardown_guard = gate.lock();

    // A concurrent emit starts while teardown holds the gate.
    let emit_mgr = Arc::clone(&mgr);
    let mut emit_task =
        tokio::spawn(async move { emit_mgr.emit("test.event", json!({ "k": "v" })).await });

    // While the gate is held, `emit` must not be able to complete: it is blocked
    // at its readiness check on the same lock teardown holds. A bare atomic flag
    // would let it read `true` and send here, past the drain below.
    assert!(
        tokio::time::timeout(Duration::from_millis(200), &mut emit_task)
            .await
            .is_err(),
        "emit must block on the readiness gate while teardown holds it"
    );

    // Teardown's clear + drain, under the held lock. `drain_pending_emits` lives
    // in the sibling `ws_loop` module; the loop replicates its try_recv sweep.
    *teardown_guard = false;
    let mut drained = 0usize;
    while rx.try_recv().is_ok() {
        drained += 1;
    }
    assert_eq!(drained, 0, "nothing was queued before teardown began");
    drop(teardown_guard);

    // With the gate released and cleared, the previously-blocked emit resumes,
    // sees `ready == false`, and rejects — enqueuing nothing onto the channel
    // the next reconnect would forward from.
    let result = tokio::time::timeout(Duration::from_secs(1), emit_task)
        .await
        .expect("emit must resume once teardown releases the gate")
        .expect("emit task must not panic");
    assert_eq!(result.unwrap_err(), "Not connected");
    assert!(
        rx.try_recv().is_err(),
        "emit must not send past the teardown drain — a message here rides the next reconnect"
    );
}

#[tokio::test]
async fn emit_with_ack_without_connection_errors_without_waiting() {
    let mgr = SocketManager::new();
    let err = mgr
        .emit_with_ack("test.event", json!({"k":"v"}), Duration::from_secs(30))
        .await
        .unwrap_err();
    assert_eq!(err, "Not connected");
}

#[tokio::test]
async fn emit_with_ack_uses_emit_queue_while_connecting() {
    // `emit_with_ack` deliberately does not gate on readiness (its delivery is
    // confirmed by the ack), so a pre-handshake channel (`ready = false`) must
    // still enqueue the frame.
    let mgr = SocketManager::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    *mgr.emit_tx.lock().await = Some(EmitChannel {
        tx,
        ready: Arc::new(Mutex::new(false)),
    });
    *mgr.shared.status.write() = ConnectionStatus::Connecting;

    let result = mgr
        .emit_with_ack("test.event", json!({"k": "v"}), Duration::from_millis(10))
        .await;

    let queued = rx
        .try_recv()
        .unwrap_or_else(|_| panic!("expected queued ACK emit, got result={result:?}"));
    assert_eq!(queued, r#"421["test.event",{"k":"v"}]"#);
    let err = result.unwrap_err();
    assert!(
        err.starts_with("Socket ack timeout for event test.event ack_id=1"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn disconnect_on_fresh_manager_is_idempotent() {
    let mgr = SocketManager::new();
    assert!(mgr.disconnect().await.is_ok());
    // Calling again must still succeed.
    assert!(mgr.disconnect().await.is_ok());
    assert_eq!(mgr.get_state().status, ConnectionStatus::Disconnected);
}

#[tokio::test]
async fn a_timed_out_socket_loop_is_aborted_and_joined() {
    use std::sync::atomic::{AtomicBool, Ordering};

    struct MarksDrop(Arc<AtomicBool>);
    impl Drop for MarksDrop {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let dropped_in_task = Arc::clone(&dropped);
    let handle = tokio::spawn(async move {
        let _guard = MarksDrop(dropped_in_task);
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;

    terminate_loop(handle, Duration::from_millis(1)).await;

    assert!(
        dropped.load(Ordering::SeqCst),
        "terminate_loop must join the aborted task before returning"
    );
}

#[tokio::test]
async fn identity_rebind_transactions_are_serialized() {
    let manager = Arc::new(SocketManager::new());
    let first = manager.lock_identity_rebind().await;

    let waiting_manager = Arc::clone(&manager);
    let mut waiter = tokio::spawn(async move {
        let _second = waiting_manager.lock_identity_rebind().await;
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(10), &mut waiter)
            .await
            .is_err(),
        "a second account rebind must not interleave with the first"
    );

    drop(first);
    tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .expect("the next rebind should proceed after the first commits")
        .unwrap();
}

#[test]
fn emit_state_change_is_safe_to_call_on_empty_shared() {
    let shared = SharedState {
        webhook_router: RwLock::new(None),
        ack_registry: AckRegistry::default(),
        status: RwLock::new(ConnectionStatus::Connecting),
        socket_id: RwLock::new(None),
        error: RwLock::new(None),
        connection_identity: RwLock::new(None),
    };
    // Must not panic even with all default state.
    emit_state_change(&shared);
}

#[test]
fn emit_server_event_is_safe_without_subscribers() {
    let shared = SharedState {
        webhook_router: RwLock::new(None),
        ack_registry: AckRegistry::default(),
        status: RwLock::new(ConnectionStatus::Connected),
        socket_id: RwLock::new(Some("x".into())),
        error: RwLock::new(None),
        connection_identity: RwLock::new(None),
    };
    // Pure logging — must not touch state or panic.
    emit_server_event(&shared, "any.event", json!({}));
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);
}

#[test]
fn set_webhook_router_populates_the_shared_slot() {
    let mgr = SocketManager::new();
    assert!(mgr.shared.webhook_router.read().is_none());
    let router = Arc::new(WebhookRouter::new(None));
    mgr.set_webhook_router(router);
    assert!(mgr.shared.webhook_router.read().is_some());
}

#[test]
fn set_webhook_router_overwrites_previous_router() {
    // Replacing the router is allowed so callers can hot-swap during
    // reconfiguration — this test nails that observable behaviour down.
    let mgr = SocketManager::new();
    mgr.set_webhook_router(Arc::new(WebhookRouter::new(None)));
    let second = Arc::new(WebhookRouter::new(None));
    let second_ptr = Arc::as_ptr(&second);
    mgr.set_webhook_router(Arc::clone(&second));
    let stored = mgr.shared.webhook_router.read().clone().unwrap();
    assert!(std::ptr::eq(Arc::as_ptr(&stored), second_ptr));
}

#[tokio::test]
async fn emit_after_disconnect_errors_not_connected() {
    // Even without ever calling connect(), the disconnect() call path
    // leaves the emit channel torn down — and emit() must reject.
    let mgr = SocketManager::new();
    mgr.disconnect().await.unwrap();
    let err = mgr.emit("x", json!({})).await.unwrap_err();
    assert_eq!(err, "Not connected");
}

/// Empty-token guard at the `SocketManager::connect` boundary:
/// the RPC caller must receive an `Err` immediately — not
/// `{"status":"Connecting"}` — so the UI can surface an actionable error.
#[tokio::test]
async fn connect_rejects_empty_token_and_returns_err() {
    let mgr = SocketManager::new();

    // Bare empty string.
    let err = mgr.connect("http://localhost:1", "").await.unwrap_err();
    assert!(
        err.contains("empty session token"),
        "expected 'empty session token' in error, got: {err}"
    );
    assert_eq!(mgr.get_state().status, ConnectionStatus::Disconnected);

    // Whitespace-only string (trim check).
    let err = mgr.connect("http://localhost:1", "   ").await.unwrap_err();
    assert!(err.contains("empty session token"), "{err}");
    assert_eq!(mgr.get_state().status, ConnectionStatus::Disconnected);
}
