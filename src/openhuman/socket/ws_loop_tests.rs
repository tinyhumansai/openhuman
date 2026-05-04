use super::*;
use parking_lot::RwLock;

fn make_shared() -> Arc<SharedState> {
    Arc::new(SharedState {
        webhook_router: RwLock::new(None),
        status: RwLock::new(ConnectionStatus::Connected),
        socket_id: RwLock::new(None),
    })
}

// ── handle_eio_message ─────────────────────────────────────────

#[test]
fn handle_eio_message_ping_sends_pong() {
    let shared = make_shared();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    handle_eio_message("2", &tx, &shared);
    let msg = rx.try_recv().expect("pong should be sent");
    assert_eq!(msg, "3");
}

#[test]
fn handle_eio_message_pong_is_ignored() {
    let shared = make_shared();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    handle_eio_message("3", &tx, &shared);
    assert!(rx.try_recv().is_err(), "pong must not trigger a reply");
}

#[test]
fn handle_eio_message_empty_is_noop() {
    let shared = make_shared();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    handle_eio_message("", &tx, &shared);
    assert!(rx.try_recv().is_err());
}

#[test]
fn handle_eio_message_message_routes_to_sio_packet() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    // `4` + `1` = Engine.IO MESSAGE + SIO DISCONNECT — should flip state.
    *shared.status.write() = ConnectionStatus::Connected;
    *shared.socket_id.write() = Some("old-sid".into());
    handle_eio_message("41", &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
    assert!(shared.socket_id.read().is_none());
}

#[test]
fn handle_eio_message_close_and_noop_do_not_panic() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_eio_message("1", &tx, &shared); // CLOSE from server
    handle_eio_message("6", &tx, &shared); // NOOP
    handle_eio_message("9", &tx, &shared); // unknown
}

// ── handle_sio_packet ──────────────────────────────────────────

#[test]
fn handle_sio_packet_event_dispatches_to_event_handler() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    *shared.status.write() = ConnectionStatus::Disconnected;
    // `2` = SIO EVENT, payload is a "ready" event → should flip to Connected.
    handle_sio_packet(r#"2["ready",{}]"#, &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);
}

#[test]
fn handle_sio_packet_event_with_unparseable_payload_is_logged_only() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    *shared.status.write() = ConnectionStatus::Disconnected;
    handle_sio_packet("2not-json", &tx, &shared);
    // Unparseable SIO events must not change status.
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

#[test]
fn handle_sio_packet_connect_reack_updates_sid() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    assert!(shared.socket_id.read().is_none());
    handle_sio_packet(r#"0{"sid":"new-sid-123"}"#, &tx, &shared);
    assert_eq!(shared.socket_id.read().as_deref(), Some("new-sid-123"));
}

#[test]
fn handle_sio_packet_connect_reack_missing_sid_is_noop() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_packet("0", &tx, &shared);
    assert!(shared.socket_id.read().is_none());
}

#[test]
fn handle_sio_packet_disconnect_flips_status_and_clears_sid() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    *shared.status.write() = ConnectionStatus::Connected;
    *shared.socket_id.write() = Some("sid-x".into());
    handle_sio_packet("1", &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
    assert!(shared.socket_id.read().is_none());
}

#[test]
fn handle_sio_packet_connect_error_does_not_panic() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_packet("4", &tx, &shared);
    handle_sio_packet(r#"4{"message":"nope"}"#, &tx, &shared);
}

#[test]
fn handle_sio_packet_empty_is_noop() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    handle_sio_packet("", &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);
}

#[test]
fn handle_sio_packet_unknown_type_is_noop() {
    let shared = make_shared();
    let (tx, _rx) = mpsc::unbounded_channel::<String>();
    *shared.status.write() = ConnectionStatus::Connected;
    handle_sio_packet("9abc", &tx, &shared);
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);
}

// ── End-to-end handshake tests against a local WS server ───────
//
// These tests drive the real `ws_loop` / `run_connection` code path
// against a hand-rolled Engine.IO/Socket.IO v4 server that lives on a
// 127.0.0.1 TCP listener. They intentionally don't touch rustls —
// `ws://` is used so the test never crosses TLS.

use futures_util::stream::SplitSink;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::accept_async;

type ServerWrite = SplitSink<tokio_tungstenite::WebSocketStream<TcpStream>, WsMessage>;

/// Spawn a single-accept EIO v4 server that:
///   * Sends EIO OPEN (`0{...}`) with fast ping timeouts.
///   * Optionally replies to the client's SIO CONNECT with `40{}`
///     (ack) or with `44{message:"..."}` (connect-error) based on
///     `connect_behavior`.
///   * After ack, relays every EIO MESSAGE text frame into `forward_tx`
///     so the test can assert on outgoing messages.
async fn spawn_mock_eio_server(
    connect_behavior: ConnectBehavior,
    forward_tx: mpsc::UnboundedSender<String>,
) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let ws = accept_async(stream).await.expect("ws accept");
        let (mut write, mut read) = ws.split();

        // 1. Send EIO OPEN (type 0) — short intervals so tests stay snappy.
        let open =
            r#"0{"sid":"mock-eio-sid","upgrades":[],"pingInterval":1000,"pingTimeout":2000}"#;
        let _ = write.send(WsMessage::Text(open.to_string())).await;

        // 2. Read client SIO CONNECT (`40{...}`) and forward it so tests
        //    can assert the token round-trip before the ack.
        if let Some(Ok(WsMessage::Text(t))) = read.next().await {
            let _ = forward_tx.send(t);
        }

        match connect_behavior {
            ConnectBehavior::Ack => {
                let _ = write
                    .send(WsMessage::Text(r#"40{"sid":"mock-sio-sid"}"#.into()))
                    .await;
                // 3. Forward any subsequent client-sent text frames for assertions.
                pump_client_to_forward(&mut write, &mut read, forward_tx).await;
            }
            ConnectBehavior::Error => {
                let _ = write
                    .send(WsMessage::Text(r#"44{"message":"nope"}"#.into()))
                    .await;
            }
            ConnectBehavior::GarbageOpenPacket => {
                unreachable!("handled in spawn_mock_server_with_bad_open")
            }
        }
        let _ = write.close().await;
    });
    addr
}

/// Variant of `spawn_mock_eio_server` that sends an invalid OPEN packet
/// so we can exercise the "EIO OPEN parse error" branch of `run_connection`.
async fn spawn_mock_bad_open_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("accept");
        let ws = accept_async(stream).await.expect("ws accept");
        let (mut write, _read) = ws.split();
        // Send a non-OPEN packet first, then a malformed OPEN to force
        // the JSON parse error path in `read_eio_open`.
        let _ = write.send(WsMessage::Text("6".into())).await; // NOOP — skipped
        let _ = write.send(WsMessage::Text("0{bad json".into())).await;
        let _ = write.close().await;
    });
    addr
}

#[derive(Clone, Copy)]
enum ConnectBehavior {
    Ack,
    Error,
    GarbageOpenPacket,
}

async fn pump_client_to_forward(
    write: &mut ServerWrite,
    read: &mut futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<TcpStream>>,
    forward_tx: mpsc::UnboundedSender<String>,
) {
    use tokio::time::{timeout, Duration};
    // Pump for up to 3s — tests tear down cleanly before then.
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < end {
        match timeout(Duration::from_millis(100), read.next()).await {
            Ok(Some(Ok(WsMessage::Text(t)))) => {
                let _ = forward_tx.send(t);
            }
            Ok(Some(Ok(WsMessage::Close(_)))) | Ok(None) => break,
            Ok(Some(Err(_))) => break,
            Ok(_) => continue,
            Err(_) => continue,
        }
    }
    let _ = write.close().await;
}

fn http_base_for(addr: std::net::SocketAddr) -> String {
    format!("http://{addr}")
}

/// Full happy-path handshake: client connects, server acks, shutdown
/// from the client side returns cleanly.
#[tokio::test]
async fn ws_loop_completes_handshake_and_shuts_down_cleanly() {
    let (fwd_tx, mut fwd_rx) = mpsc::unbounded_channel::<String>();
    let addr = spawn_mock_eio_server(ConnectBehavior::Ack, fwd_tx).await;

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let internal_tx = emit_tx.clone();
    drop(emit_tx); // we drive shutdown via the watch channel

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            "test-token".into(),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
        )
        .await;
    });

    // Wait until the client's SIO CONNECT frame reaches the mock server.
    // That proves the handshake progressed past EIO OPEN parse.
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
    loop {
        if let Ok(Some(frame)) =
            tokio::time::timeout(tokio::time::Duration::from_millis(200), fwd_rx.recv()).await
        {
            if frame.starts_with("40") && frame.contains("test-token") {
                break;
            }
        }
        if tokio::time::Instant::now() > deadline {
            panic!("SIO CONNECT frame never observed on server");
        }
    }

    // Status should be Connected after the ack.
    for _ in 0..50 {
        if *shared.status.read() == ConnectionStatus::Connected {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);

    // Trigger shutdown.
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

/// Server returns CONNECT_ERROR (type 44) — `run_connection` must return
/// `Failed`, then `ws_loop` should eventually see the shutdown signal
/// and exit without panicking.
#[tokio::test]
async fn ws_loop_handles_connect_error_and_shutdown() {
    let (fwd_tx, _fwd_rx) = mpsc::unbounded_channel::<String>();
    let addr = spawn_mock_eio_server(ConnectBehavior::Error, fwd_tx).await;

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            "t".into(),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
        )
        .await;
    });

    // Give the loop a moment to observe the CONNECT_ERROR, then shut down
    // before the reconnection backoff fires.
    tokio::time::sleep(tokio::time::Duration::from_millis(400)).await;
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

/// Malformed OPEN packet — exercises the EIO OPEN parse-error return
/// branch inside `run_connection`.
#[tokio::test]
async fn ws_loop_handles_bad_eio_open_and_shutdown() {
    let addr = spawn_mock_bad_open_server().await;

    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            http_base_for(addr),
            "t".into(),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
        )
        .await;
    });

    tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
    // End state must be Disconnected regardless of handshake failure mode.
    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
}

/// `ConnectBehavior::GarbageOpenPacket` exists as a future-proof
/// variant; keep it touched so clippy doesn't flag it as unused.
#[test]
fn connect_behavior_variants_are_distinct() {
    let b: ConnectBehavior = ConnectBehavior::GarbageOpenPacket;
    match b {
        ConnectBehavior::Ack => panic!(),
        ConnectBehavior::Error => panic!(),
        ConnectBehavior::GarbageOpenPacket => {}
    }
}
