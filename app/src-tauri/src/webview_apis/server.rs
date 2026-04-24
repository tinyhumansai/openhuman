//! WebSocket server for the webview_apis bridge.
//!
//! Binds a loopback TCP socket, accepts incoming connections (one per
//! core sidecar instance), and for each frame: decode → route → encode
//! response. Any number of concurrent requests per connection: each is
//! spawned as its own task and the responses are serialised back over
//! the shared sink via an mpsc.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use super::router;

/// Env var the Tauri host writes (before spawning core) and core reads
/// (in `src/openhuman/webview_apis/client.rs`) so both agree on the
/// port without a discovery round-trip.
pub const PORT_ENV: &str = "OPENHUMAN_WEBVIEW_APIS_PORT";

/// The port the server is bound to. `0` before `start()` resolves it.
static RESOLVED_PORT: AtomicU16 = AtomicU16::new(0);
static STARTED: OnceLock<()> = OnceLock::new();

pub fn resolved_port() -> u16 {
    RESOLVED_PORT.load(Ordering::SeqCst)
}

/// Start the server. Idempotent: after the first successful call any
/// subsequent call is a no-op. Returns the bound port.
///
/// Port selection: if `PORT_ENV` is set and non-zero, bind that port
/// (caller gets a deterministic port across runs — useful in dev);
/// otherwise bind `127.0.0.1:0` and let the OS pick.
pub async fn start() -> Result<u16, String> {
    if STARTED.get().is_some() {
        return Ok(resolved_port());
    }

    let requested = std::env::var(PORT_ENV)
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(0);

    let addr: SocketAddr = format!("127.0.0.1:{requested}")
        .parse()
        .map_err(|e| format!("[webview_apis] bad addr: {e}"))?;
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("[webview_apis] bind {addr} failed: {e}"))?;
    let bound = listener
        .local_addr()
        .map_err(|e| format!("[webview_apis] local_addr: {e}"))?;
    let port = bound.port();
    RESOLVED_PORT.store(port, Ordering::SeqCst);
    let _ = STARTED.set(());

    log::info!("[webview_apis] server listening on {bound}");

    tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer)) => {
                    log::info!("[webview_apis] accepted connection from {peer}");
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream).await {
                            log::warn!("[webview_apis] connection {peer} ended: {e}");
                        } else {
                            log::info!("[webview_apis] connection {peer} closed cleanly");
                        }
                    });
                }
                Err(e) => {
                    log::warn!("[webview_apis] accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            }
        }
    });

    Ok(port)
}

async fn handle_connection(stream: tokio::net::TcpStream) -> Result<(), String> {
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| format!("ws handshake: {e}"))?;
    let (mut sink, mut stream) = ws.split();

    // Responses from per-request tasks fan in here and are written back
    // in order. 32 is plenty — the core sidecar issues one request at a
    // time per op in the common path.
    let (tx, mut rx) = mpsc::channel::<String>(32);

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if let Err(e) = sink.send(Message::Text(msg)).await {
                log::warn!("[webview_apis] ws send failed: {e}");
                break;
            }
        }
    });

    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(text)) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let reply = handle_frame(&text).await;
                    if let Err(_e) = tx.send(reply).await {
                        log::warn!("[webview_apis] response channel closed before send");
                    }
                });
            }
            Ok(Message::Binary(_)) => {
                log::debug!("[webview_apis] ignoring binary frame");
            }
            Ok(Message::Ping(p)) => {
                // tungstenite auto-responds to Ping at the protocol layer;
                // log for visibility.
                log::trace!("[webview_apis] ping {} bytes", p.len());
            }
            Ok(Message::Close(_)) => {
                log::debug!("[webview_apis] peer requested close");
                break;
            }
            Ok(_) => {}
            Err(e) => {
                return Err(format!("ws recv: {e}"));
            }
        }
    }

    drop(tx);
    let _ = writer.await;
    Ok(())
}

async fn handle_frame(text: &str) -> String {
    let envelope: Request = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            log::warn!("[webview_apis] bad request frame: {e}");
            return encode_response(Response::error("<unknown>", format!("bad frame: {e}")));
        }
    };
    if envelope.kind != "request" {
        return encode_response(Response::error(
            &envelope.id,
            format!("unsupported envelope kind '{}'", envelope.kind),
        ));
    }
    let params = envelope.params.unwrap_or_default();
    let started = std::time::Instant::now();
    let result = router::dispatch(&envelope.method, params).await;
    let ms = started.elapsed().as_millis();
    match result {
        Ok(value) => {
            log::debug!(
                "[webview_apis] {} id={} ok in {ms}ms",
                envelope.method,
                envelope.id
            );
            encode_response(Response::ok(&envelope.id, value))
        }
        Err(e) => {
            log::warn!(
                "[webview_apis] {} id={} err in {ms}ms: {e}",
                envelope.method,
                envelope.id
            );
            encode_response(Response::error(&envelope.id, e))
        }
    }
}

fn encode_response(resp: Response) -> String {
    serde_json::to_string(&resp).unwrap_or_else(|e| {
        format!(
            r#"{{"kind":"response","id":"{}","ok":false,"error":"response encode failed: {}"}}"#,
            resp.id,
            e.to_string().replace('"', "\\\"")
        )
    })
}

// ── envelope types ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Request {
    kind: String,
    id: String,
    method: String,
    #[serde(default)]
    params: Option<Map<String, Value>>,
}

#[derive(Debug, Serialize)]
struct Response {
    kind: &'static str,
    id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

impl Response {
    fn ok(id: &str, result: Value) -> Self {
        Self {
            kind: "response",
            id: id.to_string(),
            ok: true,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: &str, error: impl Into<String>) -> Self {
        Self {
            kind: "response",
            id: id.to_string(),
            ok: false,
            result: None,
            error: Some(error.into()),
        }
    }
}
