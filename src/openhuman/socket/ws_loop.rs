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
#[path = "ws_loop_tests.rs"]
mod tests;
