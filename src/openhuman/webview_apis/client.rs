//! WebSocket client for the webview_apis bridge.
//!
//! One long-lived connection to the Tauri shell's local WebSocket
//! server. Requests are sent as JSON envelopes with a generated id;
//! matching responses resolve a `oneshot::Sender` kept in a pending
//! map.
//!
//! The client is lazy: the first [`request`] call opens the connection
//! and spawns a reader task. If the connection drops, the next request
//! reconnects.
//!
//! Port discovery: `OPENHUMAN_WEBVIEW_APIS_PORT` — set by the Tauri
//! host (`webview_apis::server::PORT_ENV`) before spawning this
//! process. If missing, requests return an actionable error so
//! operators can see the misconfiguration immediately.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message;

/// Env var the Tauri host writes before spawning core.
pub const PORT_ENV: &str = "OPENHUMAN_WEBVIEW_APIS_PORT";

/// Total time a single request will wait for a response. Gmail ops can
/// involve a DOM snapshot or a short navigate; 15s is a generous but
/// still-bounded ceiling.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

static CLIENT: OnceLock<Client> = OnceLock::new();

fn client() -> &'static Client {
    CLIENT.get_or_init(Client::new)
}

/// Send a request over the bridge and await the typed response.
///
/// The deserialization error surface is deliberately coarse — callers
/// get a single `String` error per envelope so the JSON-RPC handler
/// can propagate it verbatim.
pub async fn request<T>(method: &str, params: Map<String, Value>) -> Result<T, String>
where
    T: serde::de::DeserializeOwned,
{
    let started = std::time::Instant::now();
    tracing::debug!(%method, "[webview_apis-client] request");
    let raw = tokio::time::timeout(
        REQUEST_TIMEOUT,
        client().dispatch(method.to_string(), params),
    )
    .await
    .map_err(|_| {
        format!(
            "[webview_apis] {method}: timed out after {}s",
            REQUEST_TIMEOUT.as_secs()
        )
    })??;
    let parsed: T = serde_json::from_value(raw)
        .map_err(|e| format!("[webview_apis] {method}: response deserialize failed: {e}"))?;
    tracing::debug!(
        %method,
        ms = started.elapsed().as_millis() as u64,
        "[webview_apis-client] ok"
    );
    Ok(parsed)
}

// ── Internals ───────────────────────────────────────────────────────────

struct Client {
    next_id: AtomicU64,
    pending: Arc<Mutex<HashMap<String, oneshot::Sender<Result<Value, String>>>>>,
    sink: Arc<Mutex<Option<mpsc::Sender<String>>>>,
}

impl Client {
    fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            pending: Arc::new(Mutex::new(HashMap::new())),
            sink: Arc::new(Mutex::new(None)),
        }
    }

    async fn dispatch(&self, method: String, params: Map<String, Value>) -> Result<Value, String> {
        let id = format!("r{}", self.next_id.fetch_add(1, Ordering::SeqCst));
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), tx);

        let envelope = Request {
            kind: "request",
            id: &id,
            method: &method,
            params: &params,
        };
        let frame = serde_json::to_string(&envelope).map_err(|e| format!("encode request: {e}"))?;

        let sender = self.ensure_connected().await?;
        if let Err(e) = sender.send(frame).await {
            // Drop the pending entry so we don't leak.
            self.pending.lock().await.remove(&id);
            return Err(format!("send request: {e}"));
        }

        match rx.await {
            Ok(res) => res,
            Err(_) => Err("request cancelled (connection dropped)".into()),
        }
    }

    /// Return an mpsc::Sender that the reader loop holds. Reconnects
    /// if the previous connection is gone.
    async fn ensure_connected(&self) -> Result<mpsc::Sender<String>, String> {
        {
            let guard = self.sink.lock().await;
            if let Some(tx) = guard.as_ref() {
                if !tx.is_closed() {
                    return Ok(tx.clone());
                }
            }
        }
        // Connect under an exclusive lock so two concurrent callers
        // don't open two sockets.
        let mut guard = self.sink.lock().await;
        if let Some(tx) = guard.as_ref() {
            if !tx.is_closed() {
                return Ok(tx.clone());
            }
        }
        let port = std::env::var(PORT_ENV).map_err(|_| {
            format!(
                "[webview_apis] {PORT_ENV} not set — the Tauri shell must be running \
                 and have spawned this core process so the bridge port is inherited"
            )
        })?;
        let url = format!("ws://127.0.0.1:{port}/");
        tracing::info!(%url, "[webview_apis-client] connecting");
        let (ws, _) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| format!("[webview_apis] connect {url}: {e}"))?;
        let (mut sink, mut stream) = ws.split();

        let (tx, mut rx) = mpsc::channel::<String>(32);

        // Writer task: pull frames from rx and push them onto the ws sink.
        // On exit we must clear `self.sink` so `ensure_connected` opens a
        // fresh WS next time instead of handing out a dead sender.
        let sink_for_writer = Arc::clone(&self.sink);
        tokio::spawn(async move {
            while let Some(frame) = rx.recv().await {
                if let Err(e) = sink.send(Message::Text(frame)).await {
                    tracing::warn!(error = %e, "[webview_apis-client] ws send failed");
                    break;
                }
            }
            let _ = sink.send(Message::Close(None)).await;
            *sink_for_writer.lock().await = None;
        });

        // Reader task: decode responses and resolve pending oneshots.
        let pending = Arc::clone(&self.pending);
        let sink_for_reader = Arc::clone(&self.sink);
        tokio::spawn(async move {
            while let Some(msg) = stream.next().await {
                match msg {
                    Ok(Message::Text(text)) => match serde_json::from_str::<Response>(&text) {
                        Ok(r) => {
                            if let Some(tx) = pending.lock().await.remove(&r.id) {
                                let payload = if r.ok {
                                    Ok(r.result.unwrap_or(Value::Null))
                                } else {
                                    Err(r.error.unwrap_or_else(|| {
                                        "bridge returned ok=false with no error".into()
                                    }))
                                };
                                let _ = tx.send(payload);
                            } else {
                                tracing::warn!(
                                    id = %r.id,
                                    "[webview_apis-client] response for unknown id"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "[webview_apis-client] bad response frame"
                            );
                        }
                    },
                    Ok(Message::Close(_)) | Err(_) => {
                        tracing::info!("[webview_apis-client] connection closed");
                        break;
                    }
                    _ => {}
                }
            }
            // On exit, drop the cached sender so `ensure_connected`
            // reconnects on the next request, and fail every still-
            // pending request so callers don't hang.
            *sink_for_reader.lock().await = None;
            let mut pending = pending.lock().await;
            for (_id, tx) in pending.drain() {
                let _ = tx.send(Err("connection dropped".into()));
            }
        });

        *guard = Some(tx.clone());
        Ok(tx)
    }
}

// ── Envelope types ──────────────────────────────────────────────────────

#[derive(Serialize)]
struct Request<'a> {
    kind: &'static str,
    id: &'a str,
    method: &'a str,
    params: &'a Map<String, Value>,
}

#[derive(Deserialize)]
struct Response {
    #[allow(dead_code)]
    kind: String,
    id: String,
    ok: bool,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}
