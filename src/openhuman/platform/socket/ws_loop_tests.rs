use super::*;
use parking_lot::{Mutex, RwLock};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_tungstenite::tungstenite::http::{header::LOCATION, Response, StatusCode};

use crate::openhuman::platform::socket::token_provider::{
    is_invalid_token_error, static_token_provider,
};

fn make_shared() -> Arc<SharedState> {
    Arc::new(SharedState {
        webhook_router: RwLock::new(None),
        ack_registry: AckRegistry::default(),
        status: RwLock::new(ConnectionStatus::Connected),
        socket_id: RwLock::new(None),
        error: RwLock::new(None),
        connection_identity: RwLock::new(None),
    })
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
        let _ = write.send(WsMessage::Text(open.into())).await;

        // 2. Read client SIO CONNECT (`40{...}`) and forward it so tests
        //    can assert the token round-trip before the ack.
        if let Some(Ok(WsMessage::Text(t))) = read.next().await {
            let _ = forward_tx.send(t.to_string());
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
                let _ = forward_tx.send(t.to_string());
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

// ── Token-refresh and Invalid-token escalation (#2892) ────────────

/// Spawn a single-accept EIO v4 server that always rejects the SIO CONNECT
/// with `44{"message":"Invalid token"}`. Used to test the fast-fail path.
async fn spawn_mock_invalid_token_server() -> std::net::SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        // Accept connections in a loop so the server handles more than one
        // attempt (the retry-on-fresh-token path triggers a second connection).
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let ws = accept_async(stream).await.expect("ws accept");
                let (mut write, mut read) = ws.split();
                // 1. Send EIO OPEN.
                let open =
                    r#"0{"sid":"mock-eio","upgrades":[],"pingInterval":1000,"pingTimeout":2000}"#;
                let _ = write.send(WsMessage::Text(open.into())).await;
                // 2. Drain the SIO CONNECT frame (don't care about its content).
                let _ = read.next().await;
                // 3. Reply with CONNECT_ERROR "Invalid token".
                let _ = write
                    .send(WsMessage::Text(r#"44{"message":"Invalid token"}"#.into()))
                    .await;
                let _ = write.close().await;
            });
        }
    });
    addr
}

#[path = "ws_loop_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ws_loop_tests_part_02_tests.rs"]
mod part_02_tests;
