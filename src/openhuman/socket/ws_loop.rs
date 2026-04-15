//! WebSocket Engine.IO / Socket.IO connection loop with automatic reconnection.

use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage};

use crate::api::models::socket::ConnectionStatus;

use super::event_handlers::{handle_sio_event, parse_sio_event};
use super::manager::{emit_state_change, SharedState};
use super::types::{ConnectionOutcome, WsStream};

// ---------------------------------------------------------------------------
// Background loop
// ---------------------------------------------------------------------------

/// Background loop that manages the WebSocket connection and reconnection.
pub(super) async fn ws_loop(
    url: String,
    token: String,
    shared: Arc<SharedState>,
    mut emit_rx: mpsc::UnboundedReceiver<String>,
    mut shutdown_rx: watch::Receiver<bool>,
    internal_tx: mpsc::UnboundedSender<String>,
) {
    let mut backoff = Duration::from_millis(1000);
    let max_backoff = Duration::from_secs(30);

    loop {
        if *shutdown_rx.borrow() {
            break;
        }

        log::info!("[socket] Attempting connection...");
        *shared.status.write() = ConnectionStatus::Connecting;
        emit_state_change(&shared);

        let outcome = run_connection(
            &url,
            &token,
            &shared,
            &mut emit_rx,
            &mut shutdown_rx,
            &internal_tx,
        )
        .await;

        match outcome {
            ConnectionOutcome::Shutdown => {
                log::info!("[socket] Clean shutdown");
                break;
            }
            ConnectionOutcome::Lost(reason) => {
                log::warn!("[socket] Connection lost: {}", reason);
                backoff = Duration::from_millis(1000); // reset on established-then-lost
            }
            ConnectionOutcome::Failed(reason) => {
                log::error!("[socket] Connection failed: {}", reason);
                // keep growing backoff
            }
        }

        *shared.status.write() = ConnectionStatus::Disconnected;
        *shared.socket_id.write() = None;
        emit_state_change(&shared);

        if *shutdown_rx.borrow() {
            break;
        }

        log::info!("[socket] Reconnecting in {:?}...", backoff);
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
        }
        backoff = (backoff * 2).min(max_backoff);
    }

    log::info!("[socket] WebSocket loop exiting");
    *shared.status.write() = ConnectionStatus::Disconnected;
    *shared.socket_id.write() = None;
    emit_state_change(&shared);
}

// ---------------------------------------------------------------------------
// Single connection attempt
// ---------------------------------------------------------------------------

/// Run a single WebSocket connection through handshake and event loop.
async fn run_connection(
    url: &str,
    token: &str,
    shared: &Arc<SharedState>,
    emit_rx: &mut mpsc::UnboundedReceiver<String>,
    shutdown_rx: &mut watch::Receiver<bool>,
    internal_tx: &mpsc::UnboundedSender<String>,
) -> ConnectionOutcome {
    // 1. Build WebSocket URL (appends /socket.io/?EIO=4&transport=websocket)
    let ws_url = crate::api::socket::websocket_url(url);
    log::info!("[socket] WS URL: {}", ws_url);

    // 2. Connect via WebSocket (uses rustls TLS for wss://)
    let (ws_stream, _response) = match connect_async(&ws_url).await {
        Ok(r) => r,
        Err(e) => return ConnectionOutcome::Failed(format!("WebSocket connect: {e}")),
    };

    log::info!("[socket] WebSocket connected, starting handshake");
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // 3. Read Engine.IO OPEN packet (type 0)
    let open_data =
        match tokio::time::timeout(Duration::from_secs(10), read_eio_open(&mut ws_read)).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => return ConnectionOutcome::Failed(format!("EIO OPEN: {e}")),
            Err(_) => return ConnectionOutcome::Failed("Timeout waiting for EIO OPEN".into()),
        };

    let ping_interval = open_data
        .get("pingInterval")
        .and_then(|v| v.as_u64())
        .unwrap_or(25000);
    let ping_timeout_ms = open_data
        .get("pingTimeout")
        .and_then(|v| v.as_u64())
        .unwrap_or(20000);
    let eio_sid = open_data.get("sid").and_then(|v| v.as_str()).unwrap_or("?");
    log::info!(
        "[socket] EIO OPEN: sid={}, ping={}ms, timeout={}ms",
        eio_sid,
        ping_interval,
        ping_timeout_ms
    );

    // 4. Send Socket.IO CONNECT with auth token
    let connect_payload = json!({"token": token});
    let connect_msg = format!("40{}", serde_json::to_string(&connect_payload).unwrap());
    if let Err(e) = ws_write.send(WsMessage::Text(connect_msg)).await {
        return ConnectionOutcome::Failed(format!("Send SIO CONNECT: {e}"));
    }

    // 5. Read Socket.IO CONNECT ACK (type 40)
    let ack_data =
        match tokio::time::timeout(Duration::from_secs(10), read_sio_connect_ack(&mut ws_read))
            .await
        {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => return ConnectionOutcome::Failed(format!("SIO CONNECT: {e}")),
            Err(_) => {
                return ConnectionOutcome::Failed("Timeout waiting for SIO CONNECT ACK".into())
            }
        };

    let sio_sid = ack_data
        .get("sid")
        .and_then(|v| v.as_str())
        .map(String::from);
    log::info!("[socket] SIO CONNECT ACK: sid={:?}", sio_sid);

    // 6. Update state to Connected
    *shared.status.write() = ConnectionStatus::Connected;
    *shared.socket_id.write() = sio_sid;
    emit_state_change(shared);

    // 7. Main event loop
    let timeout_duration = Duration::from_millis(ping_interval + ping_timeout_ms + 5000);
    let mut deadline = Instant::now() + timeout_duration;

    loop {
        tokio::select! {
            msg = ws_read.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        deadline = Instant::now() + timeout_duration;
                        handle_eio_message(&text, internal_tx, shared);
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = ws_write.send(WsMessage::Pong(data)).await;
                    }
                    Some(Ok(WsMessage::Close(_))) => {
                        log::info!("[socket] Server closed WebSocket");
                        return ConnectionOutcome::Lost("Server closed connection".into());
                    }
                    Some(Err(e)) => {
                        return ConnectionOutcome::Lost(format!("WebSocket error: {e}"));
                    }
                    None => {
                        return ConnectionOutcome::Lost("WebSocket stream ended".into());
                    }
                    _ => {} // Binary, Pong, Frame
                }
            }
            outgoing = emit_rx.recv() => {
                match outgoing {
                    Some(msg) => {
                        if let Err(e) = ws_write.send(WsMessage::Text(msg)).await {
                            return ConnectionOutcome::Lost(format!("Send failed: {e}"));
                        }
                    }
                    None => {
                        let _ = ws_write.send(WsMessage::Close(None)).await;
                        return ConnectionOutcome::Shutdown;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                log::warn!(
                    "[socket] Ping timeout ({}ms)",
                    ping_interval + ping_timeout_ms + 5000
                );
                return ConnectionOutcome::Lost("Ping timeout".into());
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    log::info!("[socket] Shutdown signal received");
                    let _ = ws_write.send(WsMessage::Close(None)).await;
                    return ConnectionOutcome::Shutdown;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Handshake helpers
// ---------------------------------------------------------------------------

/// Read the Engine.IO OPEN packet (type 0) from the WebSocket.
///
/// Format: `0{"sid":"...","upgrades":[],"pingInterval":25000,"pingTimeout":20000}`
async fn read_eio_open(
    ws_read: &mut futures_util::stream::SplitStream<WsStream>,
) -> Result<serde_json::Value, String> {
    loop {
        match ws_read.next().await {
            Some(Ok(WsMessage::Text(text))) => {
                let s: &str = &text;
                if let Some(json_str) = s.strip_prefix('0') {
                    return serde_json::from_str(json_str)
                        .map_err(|e| format!("Parse EIO OPEN JSON: {e}"));
                }
                log::debug!(
                    "[socket] Skipping non-OPEN packet: {}",
                    &s[..s.len().min(40)]
                );
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WS error during handshake: {e}")),
            None => return Err("WebSocket closed before OPEN".into()),
        }
    }
}

/// Read the Socket.IO CONNECT ACK (type 40) from the WebSocket.
///
/// Format: `40{"sid":"..."}` or `44{"message":"error"}` for connect error.
async fn read_sio_connect_ack(
    ws_read: &mut futures_util::stream::SplitStream<WsStream>,
) -> Result<serde_json::Value, String> {
    loop {
        match ws_read.next().await {
            Some(Ok(WsMessage::Text(text))) => {
                let s: &str = &text;
                // Engine.IO MESSAGE (4) + Socket.IO CONNECT (0)
                if let Some(json_str) = s.strip_prefix("40") {
                    if json_str.is_empty() {
                        return Ok(json!({}));
                    }
                    return serde_json::from_str(json_str)
                        .map_err(|e| format!("Parse CONNECT ACK: {e}"));
                }
                // Engine.IO MESSAGE (4) + Socket.IO CONNECT_ERROR (4)
                if let Some(json_str) = s.strip_prefix("44") {
                    let err: serde_json::Value =
                        serde_json::from_str(json_str).unwrap_or(json!({"message": "unknown"}));
                    let msg = err
                        .get("message")
                        .and_then(|v| v.as_str())
                        .unwrap_or("Connect error");
                    return Err(format!("Socket.IO connect error: {msg}"));
                }
                // Engine.IO PING (2) — respond via log, can't write from here
                if s.starts_with('2') {
                    log::debug!("[socket] EIO ping during handshake (will respond after)");
                    continue;
                }
                log::debug!(
                    "[socket] Skipping packet during SIO handshake: {}",
                    &s[..s.len().min(40)]
                );
            }
            Some(Ok(_)) => continue,
            Some(Err(e)) => return Err(format!("WS error during SIO handshake: {e}")),
            None => return Err("WebSocket closed before CONNECT ACK".into()),
        }
    }
}

// ---------------------------------------------------------------------------
// Message handling
// ---------------------------------------------------------------------------

/// Handle an incoming Engine.IO text message by its type prefix.
fn handle_eio_message(
    text: &str,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    if text.is_empty() {
        return;
    }

    match text.as_bytes()[0] {
        b'2' => {
            // Engine.IO PING → respond with PONG
            let _ = emit_tx.send("3".to_string());
        }
        b'3' => {
            // Engine.IO PONG — ignore (server responding to our ping)
        }
        b'4' => {
            // Engine.IO MESSAGE → contains Socket.IO packet
            if text.len() > 1 {
                handle_sio_packet(&text[1..], emit_tx, shared);
            }
        }
        b'1' => {
            log::info!("[socket] Engine.IO CLOSE from server");
        }
        b'6' => {
            // Engine.IO NOOP
        }
        _ => {
            log::debug!(
                "[socket] Unknown EIO packet: {}",
                &text[..text.len().min(30)]
            );
        }
    }
}

/// Handle a Socket.IO packet (after stripping the Engine.IO '4' prefix).
fn handle_sio_packet(
    text: &str,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    if text.is_empty() {
        return;
    }

    match text.as_bytes()[0] {
        b'2' => {
            // Socket.IO EVENT: 2["eventName", data]
            if let Some((event_name, data)) = parse_sio_event(&text[1..]) {
                handle_sio_event(&event_name, data, emit_tx, shared);
            } else {
                log::warn!(
                    "[socket] Failed to parse SIO EVENT: {}",
                    &text[..text.len().min(80)]
                );
            }
        }
        b'0' => {
            // Socket.IO CONNECT (re-ack during reconnection) — update sid
            log::debug!("[socket] SIO CONNECT re-ack");
            if text.len() > 1 {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text[1..]) {
                    if let Some(sid) = data.get("sid").and_then(|v| v.as_str()) {
                        *shared.socket_id.write() = Some(sid.to_string());
                        emit_state_change(shared);
                    }
                }
            }
        }
        b'1' => {
            // Socket.IO DISCONNECT
            log::info!("[socket] SIO DISCONNECT from server");
            *shared.status.write() = ConnectionStatus::Disconnected;
            *shared.socket_id.write() = None;
            emit_state_change(shared);
        }
        b'4' => {
            // Socket.IO CONNECT_ERROR
            let error_str = if text.len() > 1 {
                &text[1..]
            } else {
                "unknown"
            };
            log::error!("[socket] SIO CONNECT_ERROR: {}", error_str);
        }
        _ => {
            log::debug!(
                "[socket] Unknown SIO packet type: {}",
                &text[..text.len().min(30)]
            );
        }
    }
}

#[cfg(test)]
mod tests {
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
}
