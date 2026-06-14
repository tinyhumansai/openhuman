//! [`CdpConn`] — per-attach handle on top of the in-process CDP transport.
//!
//! Wraps an [`Arc<WebviewCdpTransport>`](super::in_process::WebviewCdpTransport)
//! with the same `call` / `pump_events` surface the scanners and the
//! per-account session opener were already using. The previous
//! WebSocket-backed implementation (one socket per attach) is gone for
//! new code paths; all attaches for a given webview now share the same
//! in-process channel, and a [`CdpConn`] is just a cheap session-scoped
//! view.
//!
//! For backward compatibility with per-scanner duplicated implementations
//! that still attach via the TCP loopback DevTools port (whatsapp,
//! slack, telegram, wechat, meet_video — see issue follow-up), the
//! [`CdpConn::open_ws`] legacy constructor keeps the old `tungstenite`
//! WebSocket path alive. Callers should migrate to
//! [`super::conn_for_account`] over time.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::oneshot;
use tokio_tungstenite::{connect_async, tungstenite::Message};

use super::in_process::{EventFrame, WebviewCdpTransport};

/// Timeout applied to a single request/response round-trip in the
/// legacy WebSocket transport. Long enough to cover a cold-attach on a
/// sluggish machine; the in-process transport uses
/// [`crate::cdp::CALL_TIMEOUT`] (also 35s) for symmetry.
const LEGACY_CALL_TIMEOUT: Duration = Duration::from_secs(35);

/// Internal: legacy WebSocket dispatch state.
struct LegacyWs {
    sink: futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    stream: futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
    next_id: i64,
    pending: HashMap<i64, oneshot::Sender<Result<Value, String>>>,
}

enum Backend {
    InProcess(Arc<WebviewCdpTransport>),
    LegacyWs(LegacyWs),
}

/// Per-attach CDP handle. Internally either wraps an
/// `Arc<WebviewCdpTransport>` (in-process channel) or a tungstenite
/// `WebSocketStream` (legacy TCP loopback). The session_id filter is
/// per-handle so concurrent attachers don't see each other's events.
pub struct CdpConn {
    backend: Backend,
    label: String,
}

impl CdpConn {
    /// Wrap an already-installed in-process transport. Callers obtain
    /// the transport from the per-app [`super::CdpRegistry`]
    /// (`app.state()`) — typically via
    /// [`super::conn_for_account`].
    pub fn new(transport: Arc<WebviewCdpTransport>) -> Self {
        let label = transport.label().to_string();
        Self {
            backend: Backend::InProcess(transport),
            label,
        }
    }

    /// Legacy: open a CDP connection over the loopback TCP WebSocket
    /// exposed by `--remote-debugging-port`. Kept for the per-scanner
    /// duplicated implementations (whatsapp, slack, telegram, wechat,
    /// meet_video) that have not yet migrated to the in-process
    /// channel. New code paths should use [`super::conn_for_account`].
    pub async fn open_ws(ws_url: &str) -> Result<Self, String> {
        let (ws, _resp) = connect_async(ws_url)
            .await
            .map_err(|e| format!("ws connect: {e}"))?;
        let (sink, stream) = ws.split();
        Ok(Self {
            backend: Backend::LegacyWs(LegacyWs {
                sink,
                stream,
                next_id: 1,
                pending: HashMap::new(),
            }),
            label: format!("ws:{ws_url}"),
        })
    }

    /// Setup-phase request/response: sends a JSON-RPC call and awaits
    /// the matching response. `session_id`, when supplied, is inlined
    /// into the envelope so the call routes to a previously-attached
    /// child target (via `Target.attachToTarget`).
    pub async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        match &mut self.backend {
            Backend::InProcess(transport) => transport.call(method, params, session_id).await,
            Backend::LegacyWs(ws) => legacy_ws_call(ws, method, params, session_id).await,
        }
    }

    /// Subscribe to the transport's event stream and dispatch every
    /// inbound CDP event via the supplied callback until the channel
    /// signals it cannot keep up.
    ///
    /// `session_id` filters incoming events — CDP multiplexes all
    /// sessions through the same transport when `flatten: true` is set,
    /// so we drop events belonging to other sessions.
    ///
    /// Returns when the channel closes (the transport has been
    /// forgotten / ws shut down) or on an unrecoverable error.
    /// `Lagged` is treated as a continuation signal — the caller's idle
    /// watchdog will eventually time out the session and the outer
    /// reconnect loop re-attaches.
    pub async fn pump_events<F>(&mut self, session_id: &str, mut on_event: F) -> Result<(), String>
    where
        F: FnMut(&str, &Value),
    {
        match &mut self.backend {
            Backend::InProcess(transport) => {
                let mut rx = transport.subscribe_events();
                loop {
                    match rx.recv().await {
                        Ok(EventFrame {
                            method,
                            params,
                            session_id: evt_session,
                        }) => {
                            if !evt_session.is_empty() && evt_session != session_id {
                                continue;
                            }
                            on_event(&method, &params);
                        }
                        Err(RecvError::Lagged(skipped)) => {
                            log::warn!(
                                "[cdp][{}] event channel lagged skipped={} session_id={}",
                                self.label,
                                skipped,
                                session_id
                            );
                            continue;
                        }
                        Err(RecvError::Closed) => return Ok(()),
                    }
                }
            }
            Backend::LegacyWs(ws) => legacy_ws_pump_events(ws, session_id, on_event).await,
        }
    }

    /// Diagnostic helper — webview label (in-process) or
    /// `"ws:<url>"` (legacy WS) this connection is bound to.
    pub fn label(&self) -> &str {
        &self.label
    }
}

async fn legacy_ws_call(
    ws: &mut LegacyWs,
    method: &str,
    params: Value,
    session_id: Option<&str>,
) -> Result<Value, String> {
    let id = ws.next_id;
    ws.next_id += 1;
    let mut req = json!({ "id": id, "method": method, "params": params });
    if let Some(s) = session_id {
        req["sessionId"] = json!(s);
    }
    let body = serde_json::to_string(&req).map_err(|e| format!("encode: {e}"))?;
    ws.sink
        .send(Message::Text(body))
        .await
        .map_err(|e| format!("ws send: {e}"))?;
    loop {
        let msg = tokio::time::timeout(LEGACY_CALL_TIMEOUT, ws.stream.next())
            .await
            .map_err(|_| format!("ws read timeout (method={method})"))?
            .ok_or_else(|| format!("ws closed (method={method})"))?
            .map_err(|e| format!("ws recv: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                continue
            }
            Message::Close(_) => return Err("ws closed".into()),
        };
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("decode: {e}"))?;
        if v.get("id").and_then(|x| x.as_i64()) != Some(id) {
            continue;
        }
        if let Some(err) = v.get("error") {
            return Err(format!("cdp error: {err}"));
        }
        return Ok(v.get("result").cloned().unwrap_or(Value::Null));
    }
}

async fn legacy_ws_pump_events<F>(
    ws: &mut LegacyWs,
    session_id: &str,
    mut on_event: F,
) -> Result<(), String>
where
    F: FnMut(&str, &Value),
{
    loop {
        let msg = ws
            .stream
            .next()
            .await
            .ok_or_else(|| "ws closed".to_string())?
            .map_err(|e| format!("ws recv: {e}"))?;
        let text = match msg {
            Message::Text(t) => t,
            Message::Binary(_) | Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {
                continue
            }
            Message::Close(_) => return Ok(()),
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(id) = v.get("id").and_then(|x| x.as_i64()) {
            if let Some(tx) = ws.pending.remove(&id) {
                let res = if let Some(err) = v.get("error") {
                    Err(format!("cdp error: {err}"))
                } else {
                    Ok(v.get("result").cloned().unwrap_or(Value::Null))
                };
                let _ = tx.send(res);
            }
            continue;
        }
        let method = v.get("method").and_then(|x| x.as_str()).unwrap_or("");
        let evt_session = v.get("sessionId").and_then(|x| x.as_str()).unwrap_or("");
        if !evt_session.is_empty() && evt_session != session_id {
            continue;
        }
        let params = v.get("params").cloned().unwrap_or(Value::Null);
        on_event(method, &params);
    }
}
