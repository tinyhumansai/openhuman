//! SocketManager — persistent Rust-native Socket.io connection via WebSocket.
//!
//! Implements Engine.IO v4 and Socket.IO v4 protocols directly over WebSocket
//! using `tokio-tungstenite` with `rustls` TLS. This avoids the macOS
//! SecureTransport (`native-tls`) TLS errors that occurred with `tf-rust-socketio`.
//!
//! Responsibilities:
//! - MCP `listTools` / `toolCall` handled directly via the SkillRegistry (desktop only)
//! - Non-MCP server events forwarded to running skills AND to the frontend
//! - Connection state emitted to the frontend via Tauri events
//! - Automatic reconnection with exponential backoff
//!
//! Desktop runtime Socket.IO manager.

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::json;

use crate::api::models::socket::{ConnectionStatus, SocketState};

// WebSocket-based Socket.IO client (desktop)
use {
    futures_util::{SinkExt, StreamExt},
    tokio::sync::{mpsc, watch},
    tokio::time::{Duration, Instant},
    tokio_tungstenite::{connect_async, tungstenite::Message as WsMessage},
};

// SkillRegistry only available on desktop
use crate::openhuman::skills::skill_registry::SkillRegistry;
use crate::openhuman::skills::types::{SkillSnapshot, SkillStatus, ToolCallOrigin};
use crate::openhuman::webhooks::{WebhookRequest, WebhookRouter};

/// Events emitted to the frontend via Tauri.
#[allow(dead_code)]
pub mod events {
    /// Socket state changed (status, socket_id, error).
    pub const SOCKET_STATE_CHANGED: &str = "runtime:socket-state-changed";
    /// A server event was received and forwarded.
    pub const SERVER_EVENT: &str = "server:event";
}

// ---------------------------------------------------------------------------
// Shared state
// ---------------------------------------------------------------------------

struct SharedState {
    registry: RwLock<Option<Arc<SkillRegistry>>>,
    webhook_router: RwLock<Option<Arc<WebhookRouter>>>,
    status: RwLock<ConnectionStatus>,
    socket_id: RwLock<Option<String>>,
}

// ---------------------------------------------------------------------------
// WebSocket stream type alias (desktop)
// ---------------------------------------------------------------------------
type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ---------------------------------------------------------------------------
// Connection outcome (desktop)
// ---------------------------------------------------------------------------
enum ConnectionOutcome {
    /// Clean shutdown requested.
    Shutdown,
    /// Was connected then lost (reset backoff on reconnect).
    Lost(String),
    /// Failed during handshake (keep growing backoff).
    Failed(String),
}

// ---------------------------------------------------------------------------
// SocketManager
// ---------------------------------------------------------------------------

pub struct SocketManager {
    shared: Arc<SharedState>,
    emit_tx: tokio::sync::Mutex<Option<mpsc::UnboundedSender<String>>>,
    shutdown_tx: tokio::sync::Mutex<Option<watch::Sender<bool>>>,
    loop_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SocketManager {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SharedState {
                registry: RwLock::new(None),
                webhook_router: RwLock::new(None),
                status: RwLock::new(ConnectionStatus::Disconnected),
                socket_id: RwLock::new(None),
            }),
            emit_tx: tokio::sync::Mutex::new(None),
            shutdown_tx: tokio::sync::Mutex::new(None),
            loop_handle: tokio::sync::Mutex::new(None),
        }
    }

    /// Set the skill registry for MCP tool handling.
    pub fn set_registry(&self, registry: Arc<SkillRegistry>) {
        *self.shared.registry.write() = Some(registry);
    }

    /// Set the webhook router for skill-targeted webhook delivery.
    pub fn set_webhook_router(&self, router: Arc<WebhookRouter>) {
        *self.shared.webhook_router.write() = Some(router);
    }

    /// Get current socket state.
    pub fn get_state(&self) -> SocketState {
        SocketState {
            status: *self.shared.status.read(),
            socket_id: self.shared.socket_id.read().clone(),
            error: None,
        }
    }

    /// Check if connected.
    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        *self.shared.status.read() == ConnectionStatus::Connected
    }

    // -----------------------------------------------------------------------
    // Connection lifecycle (desktop)
    // -----------------------------------------------------------------------
    pub async fn connect(&self, url: &str, token: &str) -> Result<(), String> {
        self.disconnect().await?;

        log::info!("[socket-mgr] Connecting to {}", url);

        *self.shared.status.write() = ConnectionStatus::Connecting;
        Self::emit_state_change(&self.shared);

        let (emit_tx, emit_rx) = mpsc::unbounded_channel::<String>();
        let internal_tx = emit_tx.clone();
        let (shutdown_tx, shutdown_rx) = watch::channel(false);

        *self.emit_tx.lock().await = Some(emit_tx);
        *self.shutdown_tx.lock().await = Some(shutdown_tx);

        let url = url.to_string();
        let token = token.to_string();
        let shared = Arc::clone(&self.shared);

        let handle = tokio::spawn(async move {
            ws_loop(url, token, shared, emit_rx, shutdown_rx, internal_tx).await;
        });

        *self.loop_handle.lock().await = Some(handle);
        Ok(())
    }
    pub async fn disconnect(&self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.lock().await.take() {
            let _ = tx.send(true);
        }
        self.emit_tx.lock().await.take();
        if let Some(handle) = self.loop_handle.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_secs(5), handle).await;
        }
        *self.shared.status.write() = ConnectionStatus::Disconnected;
        *self.shared.socket_id.write() = None;
        Self::emit_state_change(&self.shared);
        Ok(())
    }
    pub async fn emit(&self, event: &str, data: serde_json::Value) -> Result<(), String> {
        if let Some(ref tx) = *self.emit_tx.lock().await {
            let payload =
                serde_json::to_string(&json!([event, data])).map_err(|e| format!("{e}"))?;
            let msg = format!("42{}", payload);
            tx.send(msg).map_err(|_| "Socket not connected".to_string())
        } else {
            Err("Not connected".to_string())
        }
    }

    // -----------------------------------------------------------------------
    // Tool sync — notify backend of current skill/tool state
    // -----------------------------------------------------------------------

    /// Emit `tool:sync` with the current skill/tool state.
    /// Called on socket reconnect and after skill lifecycle changes.
    pub async fn sync_tools(&self) {
        let payload = build_tool_sync_payload(&self.shared);
        if let Some(payload) = payload {
            if let Err(e) = self.emit("tool:sync", payload).await {
                log::debug!("[socket-mgr] tool:sync emit failed: {e}");
            }
        }
    }
    // -----------------------------------------------------------------------
    // Tauri event helpers
    // -----------------------------------------------------------------------

    fn emit_state_change(shared: &SharedState) {
        let status = *shared.status.read();
        let socket_id = shared.socket_id.read().clone();
        log::debug!(
            "[socket-mgr] State changed: {:?}, sid={:?}",
            status,
            socket_id
        );
    }

    fn emit_server_event(_shared: &SharedState, event_name: &str, _data: serde_json::Value) {
        log::debug!("[socket-mgr] Server event: {}", event_name);
    }
}

impl Default for SocketManager {
    fn default() -> Self {
        Self::new()
    }
}

// ===========================================================================
// WebSocket Engine.IO/Socket.IO implementation (desktop)
// ===========================================================================
async fn ws_loop(
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

        log::info!("[socket-mgr] Attempting connection...");
        *shared.status.write() = ConnectionStatus::Connecting;
        SocketManager::emit_state_change(&shared);

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
                log::info!("[socket-mgr] Clean shutdown");
                break;
            }
            ConnectionOutcome::Lost(reason) => {
                log::warn!("[socket-mgr] Connection lost: {}", reason);
                backoff = Duration::from_millis(1000); // reset on established-then-lost
            }
            ConnectionOutcome::Failed(reason) => {
                log::error!("[socket-mgr] Connection failed: {}", reason);
                // keep growing backoff
            }
        }

        *shared.status.write() = ConnectionStatus::Disconnected;
        *shared.socket_id.write() = None;
        SocketManager::emit_state_change(&shared);

        if *shutdown_rx.borrow() {
            break;
        }

        log::info!("[socket-mgr] Reconnecting in {:?}...", backoff);
        tokio::select! {
            _ = tokio::time::sleep(backoff) => {}
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() { break; }
            }
        }
        backoff = (backoff * 2).min(max_backoff);
    }

    log::info!("[socket-mgr] WebSocket loop exiting");
    *shared.status.write() = ConnectionStatus::Disconnected;
    *shared.socket_id.write() = None;
    SocketManager::emit_state_change(&shared);
}

/// Run a single WebSocket connection through handshake and event loop.
async fn run_connection(
    url: &str,
    token: &str,
    shared: &Arc<SharedState>,
    emit_rx: &mut mpsc::UnboundedReceiver<String>,
    shutdown_rx: &mut watch::Receiver<bool>,
    internal_tx: &mpsc::UnboundedSender<String>,
) -> ConnectionOutcome {
    // 1. Build WebSocket URL
    let ws_url = crate::api::socket::websocket_url(url);
    log::info!("[socket-mgr] WS URL: {}", ws_url);

    // 2. Connect via WebSocket (uses rustls TLS for wss://)
    // Auth is passed in the Socket.IO CONNECT packet, not HTTP headers.
    let (ws_stream, _response) = match connect_async(&ws_url).await {
        Ok(r) => r,
        Err(e) => return ConnectionOutcome::Failed(format!("WebSocket connect: {e}")),
    };

    log::info!("[socket-mgr] WebSocket connected, starting handshake");
    let (mut ws_write, mut ws_read) = ws_stream.split();

    // 4. Read Engine.IO OPEN packet
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
        "[socket-mgr] EIO OPEN: sid={}, ping={}ms, timeout={}ms",
        eio_sid,
        ping_interval,
        ping_timeout_ms
    );

    // 5. Send Socket.IO CONNECT with auth
    let connect_payload = json!({"token": token});
    let connect_msg = format!("40{}", serde_json::to_string(&connect_payload).unwrap());
    if let Err(e) = ws_write.send(WsMessage::Text(connect_msg)).await {
        return ConnectionOutcome::Failed(format!("Send SIO CONNECT: {e}"));
    }

    // 6. Read Socket.IO CONNECT ACK
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
    log::info!("[socket-mgr] SIO CONNECT ACK: sid={:?}", sio_sid);

    // 7. Update state: Connected
    *shared.status.write() = ConnectionStatus::Connected;
    *shared.socket_id.write() = sio_sid;
    SocketManager::emit_state_change(shared);

    // 8. Main event loop
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
                        log::info!("[socket-mgr] Server closed WebSocket");
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
            // Outgoing events from SocketManager::emit() or MCP handlers
            outgoing = emit_rx.recv() => {
                match outgoing {
                    Some(msg) => {
                        if let Err(e) = ws_write.send(WsMessage::Text(msg)).await {
                            return ConnectionOutcome::Lost(format!("Send failed: {e}"));
                        }
                    }
                    None => {
                        // Channel closed (disconnect requested)
                        let _ = ws_write.send(WsMessage::Close(None)).await;
                        return ConnectionOutcome::Shutdown;
                    }
                }
            }
            // Ping timeout — server stopped sending pings
            _ = tokio::time::sleep_until(deadline) => {
                log::warn!("[socket-mgr] Ping timeout ({}ms)", ping_interval + ping_timeout_ms + 5000);
                return ConnectionOutcome::Lost("Ping timeout".into());
            }
            // Shutdown signal
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    log::info!("[socket-mgr] Shutdown signal received");
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
                    "[socket-mgr] Skipping non-OPEN packet: {}",
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
                    log::debug!("[socket-mgr] EIO ping during handshake (will respond after)");
                    continue;
                }
                log::debug!(
                    "[socket-mgr] Skipping packet during SIO handshake: {}",
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

/// Handle an incoming Engine.IO text message.
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
            log::info!("[socket-mgr] Engine.IO CLOSE from server");
        }
        b'6' => {
            // Engine.IO NOOP
        }
        _ => {
            log::debug!(
                "[socket-mgr] Unknown EIO packet: {}",
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
            // May have ACK id: 2<id>["eventName", data]
            if let Some((event_name, data)) = parse_sio_event(&text[1..]) {
                handle_sio_event(&event_name, data, emit_tx, shared);
            } else {
                log::warn!(
                    "[socket-mgr] Failed to parse SIO EVENT: {}",
                    &text[..text.len().min(80)]
                );
            }
        }
        b'0' => {
            // Socket.IO CONNECT (re-ack during reconnection) — update state
            log::debug!("[socket-mgr] SIO CONNECT re-ack");
            if text.len() > 1 {
                if let Ok(data) = serde_json::from_str::<serde_json::Value>(&text[1..]) {
                    if let Some(sid) = data.get("sid").and_then(|v| v.as_str()) {
                        *shared.socket_id.write() = Some(sid.to_string());
                        SocketManager::emit_state_change(shared);
                    }
                }
            }
        }
        b'1' => {
            // Socket.IO DISCONNECT
            log::info!("[socket-mgr] SIO DISCONNECT from server");
            *shared.status.write() = ConnectionStatus::Disconnected;
            *shared.socket_id.write() = None;
            SocketManager::emit_state_change(shared);
        }
        b'4' => {
            // Socket.IO CONNECT_ERROR
            let error_str = if text.len() > 1 {
                &text[1..]
            } else {
                "unknown"
            };
            log::error!("[socket-mgr] SIO CONNECT_ERROR: {}", error_str);
        }
        _ => {
            log::debug!(
                "[socket-mgr] Unknown SIO packet type: {}",
                &text[..text.len().min(30)]
            );
        }
    }
}

/// Handle a Socket.IO event by name.
fn handle_sio_event(
    event_name: &str,
    data: serde_json::Value,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    match event_name {
        "ready" => {
            log::info!("[socket-mgr] Server ready — auth successful");
            *shared.status.write() = ConnectionStatus::Connected;
            SocketManager::emit_state_change(shared);

            // Sync current tool state to backend on connect/reconnect
            sync_tools_via_channel(emit_tx, shared);
        }
        "error" => {
            log::error!("[socket-mgr] Server error event: {}", data);
            *shared.status.write() = ConnectionStatus::Error;
            SocketManager::emit_state_change(shared);
        }
        // MCP handlers — desktop only
        "mcp:listTools" => {
            let shared = Arc::clone(shared);
            let tx = emit_tx.clone();
            tokio::spawn(async move {
                handle_mcp_list_tools(&shared, data, &tx).await;
            });
        }
        "mcp:toolCall" => {
            let shared = Arc::clone(shared);
            let tx = emit_tx.clone();
            tokio::spawn(async move {
                handle_mcp_tool_call(&shared, data, &tx).await;
            });
        }
        // Webhook tunnel — route to owning skill and relay response
        "webhook:request" => {
            let shared = Arc::clone(shared);
            let tx = emit_tx.clone();
            tokio::spawn(async move {
                handle_webhook_request(&shared, data, &tx).await;
            });
        }
        _ => {
            // Forward to skills (desktop only) and frontend
            {
                let shared_clone = Arc::clone(shared);
                let event_owned = event_name.to_string();
                let data_clone = data.clone();
                tokio::spawn(async move {
                    let registry = shared_clone.registry.read().clone();
                    if let Some(registry) = registry {
                        registry.broadcast_event(&event_owned, data_clone).await;
                    }
                });
            }

            SocketManager::emit_server_event(shared, event_name, data);
        }
    }
}

// ---------------------------------------------------------------------------
// MCP protocol handlers (desktop only)
// ---------------------------------------------------------------------------
async fn handle_mcp_list_tools(
    shared: &SharedState,
    data: serde_json::Value,
    emit_tx: &mpsc::UnboundedSender<String>,
) {
    let request_id = match data.get("requestId").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            log::warn!("[socket-mgr] mcp:listTools missing requestId");
            return;
        }
    };

    log::info!("[socket-mgr] mcp:listTools (requestId={})", request_id);

    let registry = shared.registry.read().clone();
    let tools: Vec<serde_json::Value> = if let Some(registry) = registry {
        registry
            .all_tools()
            .into_iter()
            .map(|(skill_id, tool)| {
                json!({
                    "name": format!("{}__{}", skill_id, tool.name),
                    "description": tool.description,
                    "inputSchema": tool.input_schema,
                })
            })
            .collect()
    } else {
        Vec::new()
    };

    log::info!("[socket-mgr] mcp:listToolsResponse — {} tools", tools.len());

    emit_via_channel(
        emit_tx,
        "mcp:listToolsResponse",
        json!({ "requestId": request_id, "tools": tools }),
    );
}
async fn handle_mcp_tool_call(
    shared: &SharedState,
    data: serde_json::Value,
    emit_tx: &mpsc::UnboundedSender<String>,
) {
    let request_id = data
        .get("requestId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let tool_call = data.get("toolCall");
    let full_name = tool_call
        .and_then(|tc| tc.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let arguments = tool_call
        .and_then(|tc| tc.get("arguments"))
        .cloned()
        .unwrap_or(json!({}));

    if request_id.is_empty() || full_name.is_empty() {
        log::warn!("[socket-mgr] mcp:toolCall — missing requestId or tool name");
        return;
    }

    log::info!(
        "[socket-mgr] mcp:toolCall {} (requestId={})",
        full_name,
        request_id
    );

    let result = match full_name.find("__") {
        Some(idx) => {
            let skill_id = &full_name[..idx];
            let tool_name = &full_name[idx + 2..];

            let registry = shared.registry.read().clone();
            if let Some(registry) = registry {
                match registry
                    .call_tool_scoped(ToolCallOrigin::External, skill_id, tool_name, arguments)
                    .await
                {
                    Ok(tool_result) => json!({
                        "content": tool_result.content,
                        "isError": tool_result.is_error,
                    }),
                    Err(e) => json!({
                        "content": [{"type": "text", "text": e}],
                        "isError": true,
                    }),
                }
            } else {
                json!({
                    "content": [{"type": "text", "text": "Skill runtime not available"}],
                    "isError": true,
                })
            }
        }
        None => {
            json!({
                "content": [{"type": "text", "text": format!(
                    "Invalid tool name: {}. Expected format: skillId__toolName",
                    full_name
                )}],
                "isError": true,
            })
        }
    };

    log::info!(
        "[socket-mgr] mcp:toolCallResponse {} (requestId={})",
        full_name,
        request_id
    );

    emit_via_channel(
        emit_tx,
        "mcp:toolCallResponse",
        json!({ "requestId": request_id, "result": result }),
    );
}

// ---------------------------------------------------------------------------
// Webhook tunnel handler
// ---------------------------------------------------------------------------

/// Handle an incoming `webhook:request` event from the backend.
///
/// Routes the request to the owning skill via the WebhookRouter, waits for the
/// skill's response, and emits `webhook:response` back through the socket.
async fn handle_webhook_request(
    shared: &SharedState,
    data: serde_json::Value,
    emit_tx: &mpsc::UnboundedSender<String>,
) {
    // Parse the incoming request
    let request: WebhookRequest = match serde_json::from_value(data.clone()) {
        Ok(r) => r,
        Err(e) => {
            log::error!("[socket-mgr] Failed to parse webhook:request payload: {e}");
            return;
        }
    };

    let correlation_id = request.correlation_id.clone();
    let tunnel_uuid = request.tunnel_uuid.clone();
    let tunnel_name = request.tunnel_name.clone();
    let method = request.method.clone();
    let path = request.path.clone();

    log::info!(
        "[socket-mgr] webhook:request {} {} (tunnel={}, correlationId={})",
        method,
        path,
        tunnel_uuid,
        correlation_id,
    );

    // Look up the owning skill via the webhook router
    let router = shared.webhook_router.read().clone();
    let skill_id = router.as_ref().and_then(|r| r.route(&tunnel_uuid));

    let (response, resolved_skill_id) = match skill_id {
        Some(sid) => {
            log::debug!(
                "[socket-mgr] webhook:request routed to skill '{}'",
                sid,
            );

            let registry = shared.registry.read().clone();
            match registry {
                Some(registry) => {
                    let result = registry
                        .send_webhook_request(
                            &sid,
                            correlation_id.clone(),
                            request.method,
                            request.path,
                            request.headers,
                            request.query,
                            request.body,
                            request.tunnel_id,
                            request.tunnel_name,
                        )
                        .await;

                    match result {
                        Ok(resp) => (resp, Some(sid)),
                        Err(e) => {
                            log::warn!(
                                "[socket-mgr] Skill webhook handler error: {}",
                                e,
                            );
                            (
                                crate::openhuman::webhooks::WebhookResponseData {
                                    correlation_id: correlation_id.clone(),
                                    status_code: 500,
                                    headers: std::collections::HashMap::new(),
                                    body: base64_encode(&format!(
                                        "{{\"error\":\"Skill handler error: {}\"}}",
                                        e.replace('"', "\\\"")
                                    )),
                                },
                                Some(sid),
                            )
                        }
                    }
                }
                None => {
                    log::warn!("[socket-mgr] No skill registry available for webhook");
                    (
                        crate::openhuman::webhooks::WebhookResponseData {
                            correlation_id: correlation_id.clone(),
                            status_code: 503,
                            headers: std::collections::HashMap::new(),
                            body: base64_encode("{\"error\":\"Runtime not ready\"}"),
                        },
                        None,
                    )
                }
            }
        }
        None => {
            log::debug!(
                "[socket-mgr] No skill registered for tunnel {}",
                tunnel_uuid,
            );
            (
                crate::openhuman::webhooks::WebhookResponseData {
                    correlation_id: correlation_id.clone(),
                    status_code: 404,
                    headers: std::collections::HashMap::new(),
                    body: base64_encode("{\"error\":\"No handler registered for this tunnel\"}"),
                },
                None,
            )
        }
    };

    // Emit webhook:response back to the backend
    emit_via_channel(
        emit_tx,
        "webhook:response",
        json!({
            "correlationId": response.correlation_id,
            "statusCode": response.status_code,
            "headers": response.headers,
            "body": response.body,
        }),
    );

    // Log activity for debugging (frontend polls activity via core RPC)
    log::info!(
        "[socket-mgr] webhook activity: {} {} → status={}, skill={:?}, tunnel={}",
        method,
        path,
        response.status_code,
        resolved_skill_id,
        tunnel_name,
    );

    log::debug!(
        "[socket-mgr] webhook:response emitted (status={})",
        response.status_code,
    );
}

/// Base64-encode a string (for webhook response bodies).
/// Uses the `STANDARD` alphabet (A-Z, a-z, 0-9, +, /) with `=` padding.
fn base64_encode(input: &str) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Send a Socket.IO event through the emit channel.
/// Formats: `42["eventName", data]`
fn emit_via_channel(tx: &mpsc::UnboundedSender<String>, event: &str, data: serde_json::Value) {
    let payload = serde_json::to_string(&json!([event, data])).unwrap_or_default();
    let msg = format!("42{}", payload);
    if let Err(e) = tx.send(msg) {
        log::error!("[socket-mgr] emit_via_channel failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// Tool sync helpers (desktop only — requires SkillRegistry)
// ---------------------------------------------------------------------------

/// Derive a unified connection status string from a Rust-side SkillSnapshot.
/// Mirrors the frontend's `deriveConnectionStatus()` logic in `src/lib/skills/hooks.ts`.
fn derive_connection_status(snap: &SkillSnapshot) -> &'static str {
    match snap.status {
        SkillStatus::Error => "error",
        SkillStatus::Pending | SkillStatus::Stopped => "offline",
        SkillStatus::Initializing => "connecting",
        SkillStatus::Stopping => "disconnected",
        SkillStatus::Running => {
            // Check the skill's self-reported connection/auth state
            let conn = snap.state.get("connection_status").and_then(|v| v.as_str());
            let auth = snap.state.get("auth_status").and_then(|v| v.as_str());

            match (conn, auth) {
                (Some("error"), _) | (_, Some("error")) => "error",
                (Some("connected"), Some("authenticated")) => "connected",
                (Some("connecting"), _) | (_, Some("authenticating")) => "connecting",
                (Some("connected"), Some("not_authenticated")) => "not_authenticated",
                (Some("disconnected"), _) => "disconnected",
                // Running with no explicit connection state = connected
                _ => "connected",
            }
        }
    }
}

/// Build the `tool:sync` payload from the current registry state.
fn build_tool_sync_payload(shared: &SharedState) -> Option<serde_json::Value> {
    let registry = shared.registry.read().clone()?;
    let skills = registry.list_skills();
    let tools: Vec<serde_json::Value> = skills
        .iter()
        .map(|snap| {
            let status = derive_connection_status(snap);
            let tool_names: Vec<String> = snap.tools.iter().map(|t| t.name.clone()).collect();
            json!({
                "skillId": snap.skill_id,
                "name": snap.name,
                "status": status,
                "tools": tool_names,
            })
        })
        .collect();
    Some(json!({ "tools": tools }))
}

/// Emit `tool:sync` synchronously via the emit channel (for use from event handlers).
fn sync_tools_via_channel(emit_tx: &mpsc::UnboundedSender<String>, shared: &SharedState) {
    if let Some(payload) = build_tool_sync_payload(shared) {
        emit_via_channel(emit_tx, "tool:sync", payload);
    }
}

// ---------------------------------------------------------------------------
// SIO event parsing
// ---------------------------------------------------------------------------

/// Parse a Socket.IO EVENT payload: `["eventName", data]` or `<ackId>["eventName", data]`.
fn parse_sio_event(text: &str) -> Option<(String, serde_json::Value)> {
    // Find the start of the JSON array (skip optional ACK id digits)
    let json_start = text.find('[')?;
    let json_str = &text[json_start..];
    let arr: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
    let event_name = arr.first()?.as_str()?.to_string();
    let data = arr.get(1).cloned().unwrap_or(serde_json::Value::Null);
    Some((event_name, data))
}
