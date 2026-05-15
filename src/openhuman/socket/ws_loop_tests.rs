use super::*;
use parking_lot::RwLock;
use tokio_tungstenite::tungstenite::http::{header::LOCATION, Response, StatusCode};

fn make_shared() -> Arc<SharedState> {
    Arc::new(SharedState {
        webhook_router: RwLock::new(None),
        status: RwLock::new(ConnectionStatus::Connected),
        socket_id: RwLock::new(None),
        error: RwLock::new(None),
    })
}

// ── Redirect resolution (the real fix for OPENHUMAN-TAURI-9X) ──

#[test]
fn resolve_redirect_upgrades_http_to_ws_for_absolute_location() {
    // Cloudflare's exact behaviour: ws://host/path → 301 Location: https://host:443/path.
    // We must rewrite https→wss so connect_async sees a WebSocket URL.
    let next = resolve_redirect_target(
        "ws://api.tinyhumans.ai/socket.io/?EIO=4&transport=websocket",
        "https://api.tinyhumans.ai:443/socket.io/?EIO=4&transport=websocket",
    )
    .expect("scheme upgrade");
    assert!(
        next.starts_with("wss://api.tinyhumans.ai"),
        "expected wss:// after upgrade, got {next}"
    );
    assert!(next.contains("/socket.io/?EIO=4&transport=websocket"));
}

#[test]
fn resolve_redirect_handles_relative_location_against_current_url() {
    // RFC 7230 allows a relative Location — must be resolved against the
    // request URL, not treated as an error.
    let next = resolve_redirect_target(
        "ws://api.example.com/socket.io/?EIO=4&transport=websocket",
        "/v2/socket.io/?EIO=4&transport=websocket",
    )
    .expect("relative resolve");
    assert_eq!(
        next,
        "ws://api.example.com/v2/socket.io/?EIO=4&transport=websocket"
    );
}

#[test]
fn resolve_redirect_preserves_ws_and_wss_schemes_verbatim() {
    let next = resolve_redirect_target("wss://a.example/socket.io/", "wss://b.example/socket.io/")
        .unwrap();
    assert!(next.starts_with("wss://b.example"));
}

#[test]
fn resolve_redirect_rejects_unsupported_scheme() {
    let err = resolve_redirect_target("wss://a.example/socket.io/", "ftp://elsewhere/socket.io/")
        .unwrap_err();
    assert!(err.contains("ftp"), "{err}");
}

#[test]
fn redirect_status_matches_only_followable_codes() {
    assert!(is_redirect_status(StatusCode::MOVED_PERMANENTLY));
    assert!(is_redirect_status(StatusCode::FOUND));
    assert!(is_redirect_status(StatusCode::TEMPORARY_REDIRECT));
    assert!(is_redirect_status(StatusCode::PERMANENT_REDIRECT));
    // 304 / 300 / 4xx / 5xx all stay errors that the backoff loop handles.
    assert!(!is_redirect_status(StatusCode::NOT_MODIFIED));
    assert!(!is_redirect_status(StatusCode::MULTIPLE_CHOICES));
    assert!(!is_redirect_status(StatusCode::BAD_REQUEST));
    assert!(!is_redirect_status(StatusCode::BAD_GATEWAY));
}

#[test]
fn extract_location_header_returns_value_when_present() {
    let resp = Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .header(LOCATION, "https://api.example.com/socket.io/")
        .body(None)
        .unwrap();
    assert_eq!(
        extract_location_header(&resp).as_deref(),
        Some("https://api.example.com/socket.io/")
    );
}

#[test]
fn extract_location_header_returns_none_when_missing() {
    let resp = Response::builder()
        .status(StatusCode::MOVED_PERMANENTLY)
        .body(None)
        .unwrap();
    assert!(extract_location_header(&resp).is_none());
}

#[test]
fn redirect_warning_is_recorded_once_and_pinned_to_first_hop() {
    // First call records original→resolved. Second call (a second hop in the
    // same attempt) must NOT overwrite — the first warning carries the
    // user-actionable signal (your configured BACKEND_URL is stale).
    let shared = make_shared();
    record_redirect_warning(
        &shared,
        "ws://api.example.com/socket.io/",
        "wss://api.example.com/socket.io/",
    );
    let first = shared.error.read().clone().unwrap();
    assert!(first.contains("ws://api.example.com"));
    assert!(first.contains("wss://api.example.com"));

    record_redirect_warning(
        &shared,
        "wss://api.example.com/socket.io/",
        "wss://api.example.com/v2/socket.io/",
    );
    let after_second = shared.error.read().clone().unwrap();
    assert_eq!(
        after_second, first,
        "second redirect must not overwrite the first warning"
    );
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

// ── log_connection_failure ─────────────────────────────────────

/// Verify that `log_connection_failure` does not panic for any call count
/// (below, at, or above the threshold). The one-shot escalation fires at
/// exactly `FAIL_ESCALATE_THRESHOLD`; above it reverts to `warn`. We can't
/// assert on log output in unit tests, but the no-panic invariant combined
/// with `fail_escalate_threshold_is_five` keeps the doc comment honest.
#[test]
fn log_connection_failure_does_not_panic_below_threshold() {
    // Calls 1 through threshold-1 stay at warn — must complete without panic.
    for i in 1..FAIL_ESCALATE_THRESHOLD {
        log_connection_failure(i, "simulated transient failure");
    }
}

#[test]
fn log_connection_failure_does_not_panic_at_and_above_threshold() {
    // Calls at and above threshold escalate to error — must also not panic.
    for i in FAIL_ESCALATE_THRESHOLD..=FAIL_ESCALATE_THRESHOLD + 3 {
        log_connection_failure(i, "simulated sustained failure");
    }
}

#[test]
fn fail_escalate_threshold_is_five() {
    // Threshold of 5 is load-bearing (doc says "~15s of accumulated backoff").
    // If the value changes the doc comment and the backoff math must be updated
    // together — this test surfaces the discrepancy immediately.
    assert_eq!(
        FAIL_ESCALATE_THRESHOLD, 5,
        "FAIL_ESCALATE_THRESHOLD changed — update the doc comment to reflect the \
         new backoff accumulation before the first Sentry event"
    );
}

/// Regression guard for OPENHUMAN-TAURI-BH: the exact wire shape the
/// sustained-outage escalation builds for an offline user
/// (`Network is unreachable (os error 51)`) must classify as a
/// network-unreachable expected error so the observability layer routes
/// it to a warn breadcrumb rather than a Sentry event. If the format
/// string in `log_connection_failure` drifts away from the substrings
/// `is_network_unreachable_message` matches on, an offline Mac will
/// start spamming Sentry again — exactly the regression this guards.
#[test]
fn sustained_outage_for_network_unreachable_classifies_as_expected() {
    use crate::core::observability::{expected_error_kind, ExpectedErrorKind};

    let reason = "WebSocket connect: IO error: Network is unreachable (os error 51)";
    let detailed = format!(
        "[socket] Connection failed (sustained outage after {FAIL_ESCALATE_THRESHOLD} attempts): {reason}"
    );
    assert_eq!(
        expected_error_kind(&detailed),
        Some(ExpectedErrorKind::NetworkUnreachable),
        "offline-user shape must classify as expected; got message: {detailed}"
    );
}

/// Counterpart: a genuine outage that lacks any of the transport-level
/// markers (e.g. a server-side HTTP 500 wrapped by tungstenite) must
/// still surface as an actionable Sentry event — i.e. not classify as
/// any expected kind. Pins the OPENHUMAN-TAURI-8M invariant ("one event
/// per sustained outage") so the BH fix doesn't accidentally silence
/// real outages.
#[test]
fn sustained_outage_for_actionable_server_error_does_not_classify() {
    use crate::core::observability::expected_error_kind;

    let reason = "SIO CONNECT: Socket.IO connect error: internal server error";
    let detailed = format!(
        "[socket] Connection failed (sustained outage after {FAIL_ESCALATE_THRESHOLD} attempts): {reason}"
    );
    assert_eq!(
        expected_error_kind(&detailed),
        None,
        "actionable outage must not be silenced; got message: {detailed}"
    );
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

/// Empty-token guard: if a caller invokes the bare `connect(url, token)` RPC
/// with an empty string the loop must bail immediately rather than spin a
/// doomed reconnect cycle that fires Sentry events on every retry. The
/// status must end up `Disconnected` and the function must return without
/// completing the reconnect loop.
#[tokio::test]
async fn ws_loop_refuses_to_start_with_empty_token() {
    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Connecting;
    *shared.socket_id.write() = Some("stale".into());

    // `_emit_tx` is kept alive so the emit channel is not closed before the
    // task starts — a closed sender would give ws_loop an `emit_rx` that
    // immediately returns `None`, potentially exiting via the Shutdown arm
    // before the empty-token guard is ever reached and masking a regression.
    let (_emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    // Shutdown channel is never signalled — if the guard fails, the test
    // will time out waiting for the spawned task to complete.
    let (_shutdown_tx, shutdown_rx) = watch::channel(false);
    let (internal_tx, _internal_rx) = mpsc::unbounded_channel::<String>();

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            // URL is deliberately invalid — if the guard misfires, the
            // task would error on connect rather than return immediately.
            "http://invalid.example.invalid:1".into(),
            "   ".into(), // whitespace-only counts as empty
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
        )
        .await;
    });

    let res = tokio::time::timeout(tokio::time::Duration::from_secs(2), handle).await;
    assert!(
        matches!(res, Ok(Ok(()))),
        "ws_loop must return cleanly on empty token (no timeout, no panic)"
    );

    assert_eq!(*shared.status.read(), ConnectionStatus::Disconnected);
    assert!(shared.socket_id.read().is_none());
}

// ── End-to-end redirect-follow (the real fix for the 301 noise) ──

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Spawn a one-shot HTTP/1.1 server that replies with a 301 redirect to
/// `location` and closes — used to prove that `connect_with_redirects`
/// follows the redirect end-to-end through `connect_async` instead of
/// surfacing the 301 as a recurring error.
async fn spawn_mock_301_redirect(location: String) -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        // Drain the incoming upgrade request so the client doesn't see RST.
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        let response = format!(
            "HTTP/1.1 301 Moved Permanently\r\n\
             Location: {location}\r\n\
             Content-Length: 0\r\n\
             Connection: close\r\n\r\n"
        );
        let _ = stream.write_all(response.as_bytes()).await;
        let _ = stream.shutdown().await;
    });
    addr
}

/// Driver-level proof: when the configured URL responds with a 301 pointing
/// at a working Engine.IO server, `ws_loop` follows the redirect, completes
/// the handshake, and records a one-shot warning in `SharedState.error` so
/// the UI can surface the stale-config signal.
#[tokio::test]
async fn ws_loop_follows_301_to_working_backend() {
    // 1. Real EIO server on `ws://127.0.0.1:PORT`.
    let (fwd_tx, mut fwd_rx) = mpsc::unbounded_channel::<String>();
    let real_addr = spawn_mock_eio_server(ConnectBehavior::Ack, fwd_tx).await;
    let real_ws_url = format!("ws://{real_addr}/socket.io/?EIO=4&transport=websocket");

    // 2. Redirect server that 301s every request to the real EIO server.
    let redirect_addr = spawn_mock_301_redirect(real_ws_url.clone()).await;

    // 3. Drive `ws_loop` with the redirect address as the base URL. This is
    //    exactly the production failure mode: BACKEND_URL points at a host
    //    that 301s the WebSocket upgrade.
    let shared = make_shared();
    *shared.status.write() = ConnectionStatus::Disconnected;
    let (emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let internal_tx = emit_tx.clone();
    drop(emit_tx);

    let loop_shared = Arc::clone(&shared);
    let handle = tokio::spawn(async move {
        ws_loop(
            format!("http://{redirect_addr}"),
            "redirect-test-token".into(),
            loop_shared,
            emit_rx,
            shutdown_rx,
            internal_tx,
        )
        .await;
    });

    // The SIO CONNECT frame arriving on the *real* server proves the redirect
    // was followed and the WebSocket handshake completed against the redirect
    // target — not the redirect host.
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(5);
    let mut saw_connect = false;
    while tokio::time::Instant::now() < deadline {
        if let Ok(Some(frame)) =
            tokio::time::timeout(tokio::time::Duration::from_millis(200), fwd_rx.recv()).await
        {
            if frame.starts_with("40") && frame.contains("redirect-test-token") {
                saw_connect = true;
                break;
            }
        }
    }
    assert!(
        saw_connect,
        "redirect was not followed — SIO CONNECT never reached the real EIO server"
    );

    for _ in 0..50 {
        if *shared.status.read() == ConnectionStatus::Connected {
            break;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    }
    assert_eq!(*shared.status.read(), ConnectionStatus::Connected);

    let warning = shared.error.read().clone();
    assert!(
        warning
            .as_deref()
            .map(|w| w.contains("redirected") && w.contains("BACKEND_URL"))
            .unwrap_or(false),
        "expected redirect warning in SharedState.error, got {warning:?}"
    );

    let _ = shutdown_tx.send(true);
    let _ = tokio::time::timeout(tokio::time::Duration::from_secs(5), handle).await;
}

/// 301 without a Location header is unrecoverable — must surface as a real
/// error and not loop forever attempting to follow nothing.
#[tokio::test]
async fn connect_with_redirects_fails_when_location_missing() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept");
        let mut buf = [0u8; 4096];
        let _ = stream.read(&mut buf).await;
        // 301 but no Location header.
        let _ = stream
            .write_all(
                b"HTTP/1.1 301 Moved Permanently\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        let _ = stream.shutdown().await;
    });

    let shared = make_shared();
    let mut url = format!("ws://{addr}/socket.io/?EIO=4&transport=websocket");
    let err = connect_with_redirects(&mut url, &shared)
        .await
        .expect_err("must surface failure when Location is absent");
    assert!(matches!(err, WsError::Http(_)));
    // No warning recorded because the redirect was never actually followed.
    assert!(shared.error.read().is_none());
}
