//! WebSocket server for the webview_apis bridge.
//!
//! Binds a loopback TCP socket, accepts incoming connections (one per
//! core sidecar instance), and for each frame: decode → route → encode
//! response. Any number of concurrent requests per connection: each is
//! spawned as its own task and the responses are serialised back over
//! the shared sink via an mpsc.

use std::net::SocketAddr;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message;

use super::router;

/// Env var the Tauri host writes (before spawning core) and core reads
/// (in `src/openhuman/webview_apis/client.rs`) so both agree on the
/// port without a discovery round-trip.
pub const PORT_ENV: &str = "OPENHUMAN_WEBVIEW_APIS_PORT";

/// The port the server is bound to. `0` before `start()` resolves it.
static RESOLVED_PORT: AtomicU16 = AtomicU16::new(0);
static STARTED: OnceLock<()> = OnceLock::new();
/// Handle to the accept loop spawned by `start()`. Held so `stop()` can
/// abort the loop on app shutdown — without this the loop owns the
/// `TcpListener` and keeps the loopback port bound past tokio runtime
/// drop, which on macOS contributes to the "abnormal exit" the OS
/// reports against the app process (issue #920).
static ACCEPT_LOOP: OnceLock<Mutex<Option<JoinHandle<()>>>> = OnceLock::new();

pub fn resolved_port() -> u16 {
    RESOLVED_PORT.load(Ordering::SeqCst)
}

/// Start the server. Idempotent: after the first successful call any
/// subsequent call is a no-op. Returns the bound port.
///
/// Port selection: always bind `127.0.0.1:0` and let the OS pick an
/// ephemeral port. The resolved port is then exported via `PORT_ENV`
/// (by the caller in `lib.rs`) so the core sidecar can discover it.
///
/// We deliberately ignore any pre-existing `PORT_ENV` value here:
/// honouring it caused Sentry OPENHUMAN-TAURI-82 on Windows — if a
/// previous run wrote `PORT_ENV=49342` into the user's environment
/// (or the env was inherited from a parent process / leftover dev
/// session), the next launch would attempt to re-bind that exact
/// port and fail with WSAEADDRINUSE / os error 10048 whenever the
/// socket was still held by another process or stuck in TIME_WAIT.
/// `PORT_ENV` is an *output* of the bridge, not an input.
pub async fn start() -> Result<u16, String> {
    if STARTED.get().is_some() {
        return Ok(resolved_port());
    }

    let addr: SocketAddr = "127.0.0.1:0"
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

    log::info!("[webview_apis] server listening on {bound} (OS-assigned ephemeral)");

    let accept_handle = tokio::spawn(async move {
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
    let slot = ACCEPT_LOOP.get_or_init(|| Mutex::new(None));
    if let Ok(mut g) = slot.lock() {
        *g = Some(accept_handle);
    }

    Ok(port)
}

/// Abort the accept loop and release the loopback port. Idempotent.
///
/// Called from the app's `RunEvent::Exit` shutdown path so the listener
/// task doesn't outlive the tokio runtime / surrounding `AppHandle` —
/// see issue #920.
pub fn stop() {
    let Some(slot) = ACCEPT_LOOP.get() else {
        return;
    };
    let handle = match slot.lock() {
        Ok(mut g) => g.take(),
        Err(_) => return,
    };
    if let Some(h) = handle {
        h.abort();
        log::info!("[webview_apis] accept loop aborted");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for Sentry OPENHUMAN-TAURI-82.
    ///
    /// If `PORT_ENV` carries a stale value pointing at a port that is
    /// already in use (the failure mode reported on Windows: a previous
    /// run wrote `49342` into the env, then the same port was held by
    /// another process / stuck in TIME_WAIT), `start()` must still
    /// succeed by binding a fresh OS-assigned ephemeral port instead of
    /// trying to re-bind the stale port.
    // Single-threaded runtime: `std::env::set_var` mutates process-global
    // state and is not thread-safe in Rust. Under the default multi-threaded
    // tokio test runtime, threads spawned by the same test could observe
    // the env between `set_var` and the restore. `current_thread` eliminates
    // that intra-test window (cross-test races between OS threads from
    // OTHER tests are still possible — see the save/restore pattern below
    // for that part). Per graycyrus review on PR #1543.
    #[tokio::test(flavor = "current_thread")]
    async fn start_ignores_stale_port_env_and_binds_ephemeral() {
        // Occupy a port so `PORT_ENV` points at something that would
        // definitely fail if `start()` honoured it.
        let blocker = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("blocker bind");
        let stale_port = blocker.local_addr().expect("blocker addr").port();
        // Save+restore `PORT_ENV` so parallel tests in the same process
        // don't see this test's mutation. (Per CodeRabbit feedback on PR
        // #1543.) `std::env::set_var` is process-global; without the
        // restore, an unrelated test asserting on `PORT_ENV` could observe
        // `stale_port` and flake.
        let prev_port_env = std::env::var(PORT_ENV).ok();
        std::env::set_var(PORT_ENV, stale_port.to_string());

        let bound = start()
            .await
            .expect("start should succeed despite stale PORT_ENV");

        assert_ne!(
            bound, stale_port,
            "start() must pick a fresh ephemeral port, not the stale one in PORT_ENV"
        );
        assert_eq!(resolved_port(), bound);

        // Hold `blocker` until after `start()` so the kernel definitely
        // can't satisfy a bind on `stale_port` — defends against the
        // exact race the Sentry issue describes.
        drop(blocker);
        // Note: `stop()` only aborts the accept loop — it does NOT (and
        // cannot) reset the `STARTED` OnceLock. If a second test in this
        // binary later calls `start()` it'll hit the idempotency
        // early-return and silently observe this test's port. A future
        // refactor to `AtomicBool`-based singleton would let `stop()`
        // fully tear down. Tracked as graycyrus feedback on PR #1543;
        // currently inert because this is the only test in the module.
        stop();
        match prev_port_env {
            Some(v) => std::env::set_var(PORT_ENV, v),
            None => std::env::remove_var(PORT_ENV),
        }
    }
}
