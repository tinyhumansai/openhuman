//! SocketManager — persistent Rust-native Socket.io connection.
//!
//! Manages the Socket.io connection from Rust (not the WebView),
//! ensuring it survives app backgrounding on all platforms.
//!
//! Responsibilities:
//! - MCP `listTools` / `toolCall` handled directly via the SkillRegistry (desktop only)
//! - Non-MCP server events forwarded to running skills AND to the frontend
//! - Connection state emitted to the frontend via Tauri events
//! - Automatic reconnection with exponential backoff
//!
//! Note: On Android, the Rust Socket.io client is not available due to
//! native-tls/OpenSSL build complexity. The frontend uses its own Socket.io
//! connection instead.

use std::sync::Arc;

use parking_lot::RwLock;
use serde_json::json;
use tauri::{AppHandle, Emitter};

use crate::models::socket::{ConnectionStatus, SocketState};

// rust_socketio only available on non-Android platforms
#[cfg(not(target_os = "android"))]
use futures_util::FutureExt;
#[cfg(not(target_os = "android"))]
use rust_socketio::{
    asynchronous::{Client, ClientBuilder},
    Event, Payload,
};

// SkillRegistry only available on desktop (V8/deno_core required)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
use crate::runtime::skill_registry::SkillRegistry;

/// Events emitted to the frontend via Tauri.
#[allow(dead_code)]
pub mod events {
    /// Socket state changed (status, socket_id, error).
    pub const SOCKET_STATE_CHANGED: &str = "runtime:socket-state-changed";
    /// A server event was received and forwarded.
    pub const SERVER_EVENT: &str = "server:event";
}

// ---------------------------------------------------------------------------
// Shared state accessible from Socket.io event callbacks
// ---------------------------------------------------------------------------

struct SharedState {
    app_handle: RwLock<Option<AppHandle>>,
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    registry: RwLock<Option<Arc<SkillRegistry>>>,
    status: RwLock<ConnectionStatus>,
    socket_id: RwLock<Option<String>>,
}

// ---------------------------------------------------------------------------
// SocketManager
// ---------------------------------------------------------------------------

/// Persistent Socket.io connection manager.
///
/// Runs the Socket.io client in Rust (not the WebView) so it survives
/// app backgrounding. On desktop, handles MCP `listTools`/`toolCall` directly
/// via the [`SkillRegistry`], and forwards other server events to running
/// skills and to the frontend.
///
/// Note: On Android, this is a stub implementation. The frontend uses its own
/// Socket.io connection instead.
pub struct SocketManager {
    shared: Arc<SharedState>,
    /// The active `rust_socketio` async client (if connected).
    /// Not available on Android due to native-tls/OpenSSL build complexity.
    #[cfg(not(target_os = "android"))]
    client: tokio::sync::Mutex<Option<Client>>,
}

impl SocketManager {
    pub fn new() -> Self {
        Self {
            shared: Arc::new(SharedState {
                app_handle: RwLock::new(None),
                #[cfg(not(any(target_os = "android", target_os = "ios")))]
                registry: RwLock::new(None),
                status: RwLock::new(ConnectionStatus::Disconnected),
                socket_id: RwLock::new(None),
            }),
            #[cfg(not(target_os = "android"))]
            client: tokio::sync::Mutex::new(None),
        }
    }

    /// Set the Tauri app handle for emitting frontend events.
    pub fn set_app_handle(&self, handle: AppHandle) {
        *self.shared.app_handle.write() = Some(handle);
    }

    /// Set the skill registry for MCP tool handling (desktop only).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn set_registry(&self, registry: Arc<SkillRegistry>) {
        *self.shared.registry.write() = Some(registry);
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
    // Connection lifecycle
    // -----------------------------------------------------------------------

    /// Connect to the server with the given URL and auth token.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub async fn connect(&self, url: &str, token: &str) -> Result<(), String> {
        // Disconnect existing connection first
        self.disconnect().await?;

        log::info!("[socket-mgr] Connecting to {}", url);

        // Update status
        *self.shared.status.write() = ConnectionStatus::Connecting;
        Self::emit_state_change(&self.shared);

        // Prepare shared-state references for the callback closures.
        // Each `.on()` handler gets its own Arc clone.
        let s_connect = Arc::clone(&self.shared);
        let s_message = Arc::clone(&self.shared);
        let s_ready = Arc::clone(&self.shared);
        let s_disconnect = Arc::clone(&self.shared);
        let s_error = Arc::clone(&self.shared);
        let s_list_tools = Arc::clone(&self.shared);
        let s_tool_call = Arc::clone(&self.shared);
        let s_any = Arc::clone(&self.shared);

        let client = ClientBuilder::new(url)
            .namespace("/")
            .auth(json!({"token": token}))
            .reconnect(true)
            .max_reconnect_attempts(0) // unlimited
            .transport_type(rust_socketio::TransportType::WebsocketUpgrade)
            // --- Connection established ---
            .on("connect", move |_payload, _client: Client| {
                let shared = Arc::clone(&s_connect);
                async move {
                    log::info!("[socket-mgr] Connected (connect event)");
                    *shared.status.write() = ConnectionStatus::Connected;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // rust_socketio v0.6 emits "message" for the namespace connect ack
            .on("message", move |payload, _client: Client| {
                let shared = Arc::clone(&s_message);
                async move {
                    log::info!("[socket-mgr] Connected (message/ack)");
                    // Only transition to connected if we're still connecting
                    let current = *shared.status.read();
                    if current != ConnectionStatus::Connected {
                        *shared.status.write() = ConnectionStatus::Connected;
                        Self::emit_state_change(&shared);
                    }
                    // Try to extract socket_id from the message payload
                    if let Payload::Text(values) = &payload {
                        if let Some(val) = values.first() {
                            if let Some(sid) = val.get("sid").and_then(|v| v.as_str()) {
                                *shared.socket_id.write() = Some(sid.to_string());
                                Self::emit_state_change(&shared);
                            }
                        }
                    }
                }
                .boxed()
            })
            // --- Server ready (auth successful) ---
            .on("ready", move |_payload, _client: Client| {
                let shared = Arc::clone(&s_ready);
                async move {
                    log::info!("[socket-mgr] Server ready — auth successful");
                    *shared.status.write() = ConnectionStatus::Connected;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // --- Disconnected ---
            .on("close", move |_payload, _client: Client| {
                let shared = Arc::clone(&s_disconnect);
                async move {
                    log::info!("[socket-mgr] Disconnected");
                    *shared.status.write() = ConnectionStatus::Disconnected;
                    *shared.socket_id.write() = None;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // --- Error ---
            .on("error", move |payload, _client: Client| {
                let shared = Arc::clone(&s_error);
                async move {
                    let msg = extract_text(&payload);
                    log::error!("[socket-mgr] Error: {}", msg);
                    *shared.status.write() = ConnectionStatus::Error;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // --- MCP: list tools ---
            .on("mcp:listTools", move |payload, client: Client| {
                let shared = Arc::clone(&s_list_tools);
                async move {
                    Self::handle_mcp_list_tools(&shared, &payload, &client).await;
                }
                .boxed()
            })
            // --- MCP: tool call ---
            .on("mcp:toolCall", move |payload, client: Client| {
                let shared = Arc::clone(&s_tool_call);
                async move {
                    Self::handle_mcp_tool_call(&shared, &payload, &client).await;
                }
                .boxed()
            })
            // --- Catch-all: forward other events to skills + frontend ---
            .on_any(move |event: Event, payload: Payload, _client: Client| {
                let shared = Arc::clone(&s_any);
                // Extract event name synchronously before entering async block
                let event_name = match &event {
                    Event::Custom(name) => name.clone(),
                    other => other.to_string(),
                };
                async move {
                    log::debug!("[socket-mgr] on_any event: {}", event_name);

                    // Skip events that already have specific handlers
                    match event_name.as_str() {
                        "connect" | "ready" | "close" | "disconnect" | "error" | "message"
                        | "mcp:listTools" | "mcp:toolCall" => return,
                        _ => {}
                    }

                    let data = extract_json(&payload).unwrap_or(serde_json::Value::Null);

                    // Forward to running skills
                    let registry = shared.registry.read().clone();
                    if let Some(registry) = registry {
                        registry.broadcast_event(&event_name, data.clone()).await;
                    }

                    // Forward to frontend
                    Self::emit_server_event(&shared, &event_name, data);
                }
                .boxed()
            })
            .connect()
            .await
            .map_err(|e| {
                log::error!("[socket-mgr] Connection error: {e}");
                format!("Socket connection failed: {e}")
            })?;

        log::info!("[socket-mgr] ClientBuilder.connect() returned successfully");

        // Store the client
        *self.client.lock().await = Some(client);

        Ok(())
    }

    /// Connect to the server with the given URL and auth token (Android stub).
    /// On Android, the Rust Socket.io client is not available due to
    /// native-tls/OpenSSL build complexity. The frontend should use its own
    /// Socket.io connection instead.
    #[cfg(target_os = "android")]
    pub async fn connect(&self, url: &str, _token: &str) -> Result<(), String> {
        log::info!(
            "[socket-mgr] Android stub - Rust Socket.io not available. URL: {}",
            url
        );
        log::info!("[socket-mgr] Frontend should use its own Socket.io connection on Android.");

        // Mark as disconnected - frontend handles its own connection
        *self.shared.status.write() = ConnectionStatus::Disconnected;
        Self::emit_state_change(&self.shared);

        // Return Ok so the app doesn't fail - socket is handled by frontend on Android
        Ok(())
    }

    /// Connect to the server with the given URL and auth token (iOS version).
    /// MCP skill handlers are not available on mobile.
    #[cfg(target_os = "ios")]
    pub async fn connect(&self, url: &str, token: &str) -> Result<(), String> {
        // Disconnect existing connection first
        self.disconnect().await?;

        log::info!("[socket-mgr] Connecting to {} (iOS)", url);

        // Update status
        *self.shared.status.write() = ConnectionStatus::Connecting;
        Self::emit_state_change(&self.shared);

        // Prepare shared-state references for the callback closures.
        let s_connect = Arc::clone(&self.shared);
        let s_message = Arc::clone(&self.shared);
        let s_ready = Arc::clone(&self.shared);
        let s_disconnect = Arc::clone(&self.shared);
        let s_error = Arc::clone(&self.shared);
        let s_any = Arc::clone(&self.shared);

        let client = ClientBuilder::new(url)
            .namespace("/")
            .auth(json!({"token": token}))
            .reconnect(true)
            .max_reconnect_attempts(0) // unlimited
            .transport_type(rust_socketio::TransportType::WebsocketUpgrade)
            // --- Connection established ---
            .on("connect", move |_payload, _client: Client| {
                let shared = Arc::clone(&s_connect);
                async move {
                    log::info!("[socket-mgr] Connected (connect event)");
                    *shared.status.write() = ConnectionStatus::Connected;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // rust_socketio v0.6 emits "message" for the namespace connect ack
            .on("message", move |payload, _client: Client| {
                let shared = Arc::clone(&s_message);
                async move {
                    log::info!("[socket-mgr] Connected (message/ack)");
                    let current = *shared.status.read();
                    if current != ConnectionStatus::Connected {
                        *shared.status.write() = ConnectionStatus::Connected;
                        Self::emit_state_change(&shared);
                    }
                    if let Payload::Text(values) = &payload {
                        if let Some(val) = values.first() {
                            if let Some(sid) = val.get("sid").and_then(|v| v.as_str()) {
                                *shared.socket_id.write() = Some(sid.to_string());
                                Self::emit_state_change(&shared);
                            }
                        }
                    }
                }
                .boxed()
            })
            // --- Server ready (auth successful) ---
            .on("ready", move |_payload, _client: Client| {
                let shared = Arc::clone(&s_ready);
                async move {
                    log::info!("[socket-mgr] Server ready — auth successful");
                    *shared.status.write() = ConnectionStatus::Connected;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // --- Disconnected ---
            .on("close", move |_payload, _client: Client| {
                let shared = Arc::clone(&s_disconnect);
                async move {
                    log::info!("[socket-mgr] Disconnected");
                    *shared.status.write() = ConnectionStatus::Disconnected;
                    *shared.socket_id.write() = None;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // --- Error ---
            .on("error", move |payload, _client: Client| {
                let shared = Arc::clone(&s_error);
                async move {
                    let msg = extract_text(&payload);
                    log::error!("[socket-mgr] Error: {}", msg);
                    *shared.status.write() = ConnectionStatus::Error;
                    Self::emit_state_change(&shared);
                }
                .boxed()
            })
            // --- Catch-all: forward events to frontend (no skills on mobile) ---
            .on_any(move |event: Event, payload: Payload, _client: Client| {
                let shared = Arc::clone(&s_any);
                let event_name = match &event {
                    Event::Custom(name) => name.clone(),
                    other => other.to_string(),
                };
                async move {
                    log::debug!("[socket-mgr] on_any event: {}", event_name);

                    // Skip events that already have specific handlers
                    match event_name.as_str() {
                        "connect" | "ready" | "close" | "disconnect" | "error" | "message" => return,
                        _ => {}
                    }

                    let data = extract_json(&payload).unwrap_or(serde_json::Value::Null);

                    // Forward to frontend only (no skill registry on mobile)
                    Self::emit_server_event(&shared, &event_name, data);
                }
                .boxed()
            })
            .connect()
            .await
            .map_err(|e| {
                log::error!("[socket-mgr] Connection error: {e}");
                format!("Socket connection failed: {e}")
            })?;

        log::info!("[socket-mgr] ClientBuilder.connect() returned successfully");

        // Store the client
        *self.client.lock().await = Some(client);

        Ok(())
    }

    /// Disconnect from the server.
    #[cfg(not(target_os = "android"))]
    pub async fn disconnect(&self) -> Result<(), String> {
        let mut client_guard = self.client.lock().await;
        if let Some(client) = client_guard.take() {
            let _ = client.disconnect().await;
        }
        *self.shared.status.write() = ConnectionStatus::Disconnected;
        *self.shared.socket_id.write() = None;
        Self::emit_state_change(&self.shared);
        Ok(())
    }

    /// Disconnect from the server (Android stub).
    #[cfg(target_os = "android")]
    pub async fn disconnect(&self) -> Result<(), String> {
        *self.shared.status.write() = ConnectionStatus::Disconnected;
        *self.shared.socket_id.write() = None;
        Self::emit_state_change(&self.shared);
        Ok(())
    }

    /// Emit an event through the Rust socket to the server.
    #[cfg(not(target_os = "android"))]
    pub async fn emit(&self, event: &str, data: serde_json::Value) -> Result<(), String> {
        let client_guard = self.client.lock().await;
        if let Some(ref client) = *client_guard {
            client
                .emit(event, data)
                .await
                .map_err(|e| format!("Failed to emit '{}': {e}", event))?;
            Ok(())
        } else {
            Err("Not connected".to_string())
        }
    }

    /// Emit an event through the Rust socket to the server (Android stub).
    #[cfg(target_os = "android")]
    pub async fn emit(&self, _event: &str, _data: serde_json::Value) -> Result<(), String> {
        Err("Rust Socket.io not available on Android. Use frontend socket.".to_string())
    }

    // -----------------------------------------------------------------------
    // Tauri event helpers
    // -----------------------------------------------------------------------

    /// Emit a socket state change event to the frontend.
    fn emit_state_change(shared: &SharedState) {
        if let Some(ref app) = *shared.app_handle.read() {
            let state = SocketState {
                status: *shared.status.read(),
                socket_id: shared.socket_id.read().clone(),
                error: None,
            };
            let _ = app.emit(events::SOCKET_STATE_CHANGED, &state);
        }
    }

    /// Emit a forwarded server event to the frontend.
    fn emit_server_event(shared: &SharedState, event_name: &str, data: serde_json::Value) {
        if let Some(ref app) = *shared.app_handle.read() {
            let _ = app.emit(
                events::SERVER_EVENT,
                json!({ "event": event_name, "data": data }),
            );
        }
    }

    // -----------------------------------------------------------------------
    // MCP protocol handlers (desktop only)
    // -----------------------------------------------------------------------

    /// Handle `mcp:listTools` — return all tools from all running skills.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    async fn handle_mcp_list_tools(shared: &SharedState, payload: &Payload, client: &Client) {
        let request_id = match extract_json(payload) {
            Some(data) => data
                .get("requestId")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            None => None,
        };

        let request_id = match request_id {
            Some(id) => id,
            None => {
                log::warn!("[socket-mgr] mcp:listTools missing requestId");
                return;
            }
        };

        log::info!("[socket-mgr] mcp:listTools (requestId={})", request_id);

        // Clone the Arc to avoid holding the parking_lot lock across await
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

        log::info!(
            "[socket-mgr] mcp:listToolsResponse — {} tools",
            tools.len()
        );

        if let Err(e) = client
            .emit(
                "mcp:listToolsResponse",
                json!({ "requestId": request_id, "tools": tools }),
            )
            .await
        {
            log::error!("[socket-mgr] Failed to emit listToolsResponse: {e}");
        }
    }

    /// Handle `mcp:toolCall` — parse `skillId__toolName` and execute.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    async fn handle_mcp_tool_call(shared: &SharedState, payload: &Payload, client: &Client) {
        let data = match extract_json(payload) {
            Some(d) => d,
            None => {
                log::warn!("[socket-mgr] mcp:toolCall — invalid payload");
                return;
            }
        };

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

        // Parse "skillId__toolName" (double underscore separator)
        let result = match full_name.find("__") {
            Some(idx) => {
                let skill_id = &full_name[..idx];
                let tool_name = &full_name[idx + 2..];

                // Clone Arc to avoid holding lock across await
                let registry = shared.registry.read().clone();
                if let Some(registry) = registry {
                    match registry.call_tool(skill_id, tool_name, arguments).await {
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

        if let Err(e) = client
            .emit(
                "mcp:toolCallResponse",
                json!({ "requestId": request_id, "result": result }),
            )
            .await
        {
            log::error!("[socket-mgr] Failed to emit toolCallResponse: {e}");
        }
    }
}

impl Default for SocketManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Payload helpers (not needed on Android)
// ---------------------------------------------------------------------------

/// Extract the first JSON value from a Socket.io payload.
#[cfg(not(target_os = "android"))]
fn extract_json(payload: &Payload) -> Option<serde_json::Value> {
    match payload {
        Payload::Text(values) => values.first().cloned(),
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

/// Extract a human-readable string from a Socket.io payload.
#[cfg(not(target_os = "android"))]
fn extract_text(payload: &Payload) -> String {
    match payload {
        Payload::Text(values) => values
            .first()
            .map(|v| v.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        #[allow(unreachable_patterns)]
        _ => "unknown".to_string(),
    }
}
