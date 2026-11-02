use super::*;
use crate::socket::sdk_token_provider;
use openhuman_embed::__host::platform::socket::SocketPort;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

#[test]
fn sdk_token_callback_refreshes_each_attempt_and_stops_when_provider_fails() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = Arc::clone(&calls);
    let provider: openhuman_embed::__host::platform::socket::TokenProvider = Arc::new(move || {
        let call = provider_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if call < 2 {
                Ok(format!("token-{call}"))
            } else {
                Err("session disappeared".into())
            }
        })
    });
    let sdk_provider = sdk_token_provider(provider);
    assert_eq!(sdk_provider(), Some("token-0".to_string()));
    assert_eq!(sdk_provider(), Some("token-1".to_string()));
    assert_eq!(sdk_provider(), None);
    assert_eq!(calls.load(Ordering::SeqCst), 3);
}

#[test]
fn sdk_token_callback_rejects_empty_credentials() {
    let provider: openhuman_embed::__host::platform::socket::TokenProvider =
        Arc::new(|| Box::pin(async { Ok("  ".to_owned()) }));
    assert_eq!(sdk_token_provider(provider)(), None);
}

#[test]
fn socket_identity_match_requires_both_url_and_session_token() {
    use openhuman_embed::__host::platform::socket::{ConnectionStatus, SocketState};

    let port = SdkSocketPort::default();
    *port.identity.write() = Some(("https://api.example".into(), "session-a".into()));
    port.transport_connected.store(true, Ordering::Release);
    *port.state.write() = SocketState {
        status: ConnectionStatus::Connected,
        socket_id: None,
        error: None,
    };
    assert!(port.is_live_for("https://api.example", "session-a"));
    assert!(!port.is_live_for("https://api.example", "session-b"));
    assert!(!port.is_live_for("https://other.example", "session-a"));
}

#[test]
fn socket_state_uses_the_socket_io_connect_sid() {
    let mut state = SocketState::default();
    let event = SocketEvent::new(
        "connect",
        tinyhumans_sdk::socket::SocketPayload::Json(vec![serde_json::json!({
            "sid": "socket-io-sid"
        })]),
    );

    apply_connect_event(&mut state, &event);

    assert_eq!(state.status, ConnectionStatus::Connected);
    assert_eq!(state.socket_id.as_deref(), Some("socket-io-sid"));
}

#[test]
fn backend_and_supervisor_errors_have_distinct_connection_effects() {
    use tinyhumans_sdk::socket::SocketPayload;

    let mut state = SocketState {
        status: ConnectionStatus::Connected,
        socket_id: Some("socket-io-sid".to_owned()),
        error: None,
    };
    let transport_connected = AtomicBool::new(true);
    let loop_active = AtomicBool::new(true);
    let stopped_on_failure = AtomicBool::new(false);
    let backend_error = SocketEvent::new(
        "error",
        SocketPayload::Json(vec![serde_json::json!({"msg": "backend failure"})]),
    );

    assert!(!apply_supervisor_event(
        &backend_error,
        &mut state,
        &transport_connected,
        &loop_active,
        &stopped_on_failure,
        false,
    ));
    assert!(apply_backend_status_event(&backend_error, &mut state));
    assert_eq!(state.status, ConnectionStatus::Error);
    assert_eq!(state.error.as_deref(), Some(r#"{"msg":"backend failure"}"#));
    assert!(transport_connected.load(Ordering::Acquire));

    let supervisor_error = SocketEvent::supervisor(
        "error",
        SocketPayload::Json(vec![serde_json::json!("socket retry failed")]),
    );
    assert!(apply_supervisor_event(
        &supervisor_error,
        &mut state,
        &transport_connected,
        &loop_active,
        &stopped_on_failure,
        false,
    ));
    assert_eq!(state.status, ConnectionStatus::Reconnecting);
    assert_eq!(state.error.as_deref(), Some("socket retry failed"));
    assert!(!transport_connected.load(Ordering::Acquire));
}

#[test]
fn supervisor_events_are_not_dispatched_as_backend_domain_events() {
    use tinyhumans_sdk::socket::SocketPayload;

    let backend = SocketEvent::new("error", SocketPayload::Json(vec![serde_json::json!({})]));
    let supervisor = SocketEvent::supervisor(
        "reconnect:escalated",
        SocketPayload::Json(vec![serde_json::json!({"attempts": 5})]),
    );

    assert!(should_dispatch_to_core(&backend));
    assert!(!should_dispatch_to_core(&supervisor));
}

#[tokio::test]
async fn disconnected_port_rejects_emits_and_disconnect_is_idempotent() {
    let port = SdkSocketPort::default();
    assert!(port
        .emit("test:event", serde_json::json!({}))
        .await
        .is_err());
    assert!(port
        .emit_with_ack(
            "test:event",
            serde_json::json!({}),
            std::time::Duration::from_millis(1),
        )
        .await
        .is_err());
    port.disconnect().await.unwrap();
    assert!(!port.is_loop_active());
    assert!(!port.loop_stopped_on_failure());
}

#[tokio::test]
async fn backend_error_state_still_allows_emit_over_live_transport() {
    use tinyhumans_sdk::socket::SocketPayload;
    use tokio::sync::{mpsc, oneshot, watch};

    let port = SdkSocketPort::default();
    port.transport_connected.store(true, Ordering::Release);
    let backend_error = SocketEvent::new(
        "error",
        SocketPayload::Json(vec![serde_json::json!({"msg": "backend failure"})]),
    );
    assert!(apply_backend_status_event(
        &backend_error,
        &mut port.state.write()
    ));
    assert_eq!(port.get_state().status, ConnectionStatus::Error);

    let (commands, mut receiver) = mpsc::channel(1);
    let (shutdown, _shutdown_receiver) = watch::channel(false);
    let (_done, done_receiver) = oneshot::channel();
    *port.actor.lock().await = Some(SocketActorHandle {
        commands,
        shutdown,
        done: done_receiver,
    });
    let responder = tokio::spawn(async move {
        let SocketCommand::Emit { event, result, .. } = receiver.recv().await.unwrap() else {
            panic!("expected ordinary emit after backend error");
        };
        assert_eq!(event, "test:event");
        result.send(Ok(())).unwrap();
    });

    port.emit("test:event", serde_json::json!({"ok": true}))
        .await
        .unwrap();
    responder.await.unwrap();
}

#[tokio::test]
async fn actor_shutdown_fails_an_emit_queued_during_reconnect_promptly() {
    use tinyhumans_sdk::TinyHumansClient;
    use tokio::sync::{mpsc, oneshot, watch};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let connection = TinyHumansClient::new(format!("http://{address}"))
        .connect_socket_with_token_provider(|| Some("token".to_owned()))
        .await
        .unwrap();

    let (commands, receiver) = mpsc::channel(4);
    let (shutdown, shutdown_receiver) = watch::channel(false);
    let (done, done_receiver) = oneshot::channel();
    let state = Arc::new(RwLock::new(SocketState::default()));
    let loop_active = Arc::new(AtomicBool::new(true));
    let transport_connected = Arc::new(AtomicBool::new(false));
    let stopped_on_failure = Arc::new(AtomicBool::new(false));
    tokio::spawn(run_socket_actor(
        connection,
        receiver,
        shutdown_receiver,
        done,
        state,
        transport_connected,
        Arc::clone(&loop_active),
        stopped_on_failure,
    ));

    let (result, response) = oneshot::channel();
    commands
        .send(SocketCommand::Emit {
            event: "test:event".to_owned(),
            data: serde_json::json!({}),
            result,
        })
        .await
        .unwrap();
    tokio::task::yield_now().await;
    shutdown.send_replace(true);

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), response)
            .await
            .expect("the queued emit waiter must be released during shutdown")
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_millis(250), done_receiver)
        .await
        .expect("the actor must stop during shutdown")
        .unwrap();
    assert!(!loop_active.load(Ordering::Acquire));
}

#[tokio::test]
async fn actor_shutdown_fails_an_ack_queued_during_reconnect_promptly() {
    use tinyhumans_sdk::TinyHumansClient;
    use tokio::sync::{mpsc, oneshot, watch};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    let connection = TinyHumansClient::new(format!("http://{address}"))
        .connect_socket_with_token_provider(|| Some("token".to_owned()))
        .await
        .unwrap();

    let (commands, receiver) = mpsc::channel(4);
    let (shutdown, shutdown_receiver) = watch::channel(false);
    let (done, done_receiver) = oneshot::channel();
    let state = Arc::new(RwLock::new(SocketState::default()));
    let loop_active = Arc::new(AtomicBool::new(true));
    let transport_connected = Arc::new(AtomicBool::new(false));
    let stopped_on_failure = Arc::new(AtomicBool::new(false));
    tokio::spawn(run_socket_actor(
        connection,
        receiver,
        shutdown_receiver,
        done,
        state,
        transport_connected,
        Arc::clone(&loop_active),
        stopped_on_failure,
    ));

    let (result, response) = oneshot::channel();
    commands
        .send(SocketCommand::EmitWithAck {
            event: "test:event".to_owned(),
            data: serde_json::json!({}),
            timeout: std::time::Duration::from_secs(30),
            result,
        })
        .await
        .unwrap();
    tokio::task::yield_now().await;
    shutdown.send_replace(true);

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(250), response)
            .await
            .expect("the queued ACK waiter must be released during shutdown")
            .unwrap()
            .is_err()
    );
    tokio::time::timeout(std::time::Duration::from_millis(250), done_receiver)
        .await
        .expect("the actor must stop during shutdown")
        .unwrap();
    assert!(!loop_active.load(Ordering::Acquire));
}
