//! CDP WebSocket client. Supports both short-lived request/response ticks
//! (whatsapp / slack / telegram periodic scans) and long-lived streaming
//! sessions with a pending-id table (discord MITM, and the new per-account
//! session opener).
//!
//! Not re-entrant: `call` is sequential during the setup phase, and once
//! `pump_events` takes over the read stream callers issue follow-up calls
//! via the pending-table machinery (TODO — V1.5, not needed yet).

use std::collections::HashMap;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio_tungstenite::{connect_async, tungstenite::Message};

/// Timeout applied to a single request/response round-trip during the setup
/// phase. Long enough to cover a cold-attach on a sluggish machine;
/// `pump_events` uses no timeout since CDP events can arrive hours apart.
const CALL_TIMEOUT: Duration = Duration::from_secs(35);

pub struct CdpConn {
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

impl CdpConn {
    pub async fn open(ws_url: &str) -> Result<Self, String> {
        let (ws, _resp) = connect_async(ws_url)
            .await
            .map_err(|e| format!("ws connect: {e}"))?;
        let (sink, stream) = ws.split();
        Ok(Self {
            sink,
            stream,
            next_id: 1,
            pending: HashMap::new(),
        })
    }

    /// Setup-phase request/response: sends a JSON-RPC call and drains inbound
    /// messages until the matching response arrives. Unrelated events and
    /// responses for other ids are dropped on the floor — only safe before
    /// `pump_events` takes over the read side.
    pub async fn call(
        &mut self,
        method: &str,
        params: Value,
        session_id: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut req = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session_id {
            req["sessionId"] = json!(s);
        }
        let body = serde_json::to_string(&req).map_err(|e| format!("encode: {e}"))?;
        self.sink
            .send(Message::Text(body))
            .await
            .map_err(|e| format!("ws send: {e}"))?;

        loop {
            let msg = tokio::time::timeout(CALL_TIMEOUT, self.stream.next())
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

    /// Take over the read stream and dispatch every inbound event via the
    /// supplied callback until the WebSocket closes. Responses to outstanding
    /// `call` requests (none in V1) route through `pending`.
    ///
    /// `session_id` filters incoming events: CDP multiplexes all sessions
    /// through one ws once `flatten: true` is set, so we drop events
    /// belonging to other sessions.
    pub async fn pump_events<F>(&mut self, session_id: &str, mut on_event: F) -> Result<(), String>
    where
        F: FnMut(&str, &Value),
    {
        loop {
            let msg = self
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
                if let Some(tx) = self.pending.remove(&id) {
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
}
