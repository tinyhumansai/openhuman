//! SocketManager — persistent Rust-native Socket.IO connection via WebSocket.
//!
//! Implements Engine.IO v4 and Socket.IO v4 protocols directly over WebSocket
//! using `tokio-tungstenite` with `rustls` TLS.
//!
//! Responsibilities:
//! - MCP `listTools` / `toolCall` handled directly via the SkillRegistry
//! - Non-MCP server events forwarded to running skills and to the frontend
//! - Connection state logging for observability
//! - Automatic reconnection with exponential backoff

use std::sync::{Arc, OnceLock};

use parking_lot::RwLock;
use serde_json::json;
use tokio::sync::{mpsc, watch};
use tokio::time::Duration;

use crate::api::models::socket::{ConnectionStatus, SocketState};
use crate::openhuman::webhooks::WebhookRouter;

use super::ws_loop::ws_loop;

// ---------------------------------------------------------------------------
// Global accessor
// ---------------------------------------------------------------------------

static GLOBAL_SOCKET_MANAGER: OnceLock<Arc<SocketManager>> = OnceLock::new();

/// Register the global `SocketManager` instance (called once during bootstrap).
pub fn set_global_socket_manager(mgr: Arc<SocketManager>) {
    if GLOBAL_SOCKET_MANAGER.set(mgr).is_err() {
        log::warn!("[socket] global SocketManager already set — ignoring duplicate");
    }
}

/// Retrieve the global `SocketManager`, if initialized.
pub fn global_socket_manager() -> Option<&'static Arc<SocketManager>> {
    GLOBAL_SOCKET_MANAGER.get()
}

// ---------------------------------------------------------------------------
// Shared state (visible to sibling modules)
// ---------------------------------------------------------------------------

/// State shared between the `SocketManager` handle and the background loop.
pub(super) struct SharedState {
    /// Router for delivering incoming webhooks to skills.
    pub(super) webhook_router: RwLock<Option<Arc<WebhookRouter>>>,
    /// Current connection status.
    pub(super) status: RwLock<ConnectionStatus>,
    /// Socket ID assigned by the server.
    pub(super) socket_id: RwLock<Option<String>>,
}

// ---------------------------------------------------------------------------
// SocketManager
// ---------------------------------------------------------------------------

/// Manages a persistent Socket.IO connection to the backend.
///
/// Handles protocol-level handshakes (Engine.IO / Socket.IO), heartbeats, and
/// automatic reconnection while providing a high-level API for emitting events
/// and syncing tool state.
pub struct SocketManager {
    /// Shared state accessible from both the manager and the background loop.
    pub(super) shared: Arc<SharedState>,
    /// Channel for sending outgoing messages to the background loop.
    emit_tx: tokio::sync::Mutex<Option<mpsc::UnboundedSender<String>>>,
    /// Channel for signaling the background loop to shut down.
    shutdown_tx: tokio::sync::Mutex<Option<watch::Sender<bool>>>,
    /// Join handle for the background connection loop.
    loop_handle: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl SocketManager {
    /// Create a new, disconnected SocketManager.
    pub fn new() -> Self {
        log::debug!("[socket] SocketManager created (disconnected)");
        Self {
            shared: Arc::new(SharedState {
                webhook_router: RwLock::new(None),
                status: RwLock::new(ConnectionStatus::Disconnected),
                socket_id: RwLock::new(None),
            }),
            emit_tx: tokio::sync::Mutex::new(None),
            shutdown_tx: tokio::sync::Mutex::new(None),
            loop_handle: tokio::sync::Mutex::new(None),
        }
    }

    /// Set the webhook router for skill-targeted webhook delivery.
    pub fn set_webhook_router(&self, router: Arc<WebhookRouter>) {
        log::debug!("[socket] WebhookRouter attached");
        *self.shared.webhook_router.write() = Some(router);
    }

    /// Get the current socket state (status, ID, error).
    pub fn get_state(&self) -> SocketState {
        SocketState {
            status: *self.shared.status.read(),
            socket_id: self.shared.socket_id.read().clone(),
            error: None,
        }
    }

    /// Check if the socket is currently connected.
    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        *self.shared.status.read() == ConnectionStatus::Connected
    }

    // -----------------------------------------------------------------------
    // Connection lifecycle
    // -----------------------------------------------------------------------

    /// Connect to the specified URL using the provided authentication token.
    ///
    /// Spawns a background `ws_loop` that manages the connection with automatic
    /// reconnection and exponential backoff.
    pub async fn connect(&self, url: &str, token: &str) -> Result<(), String> {
        self.disconnect().await?;

        log::info!("[socket] Connecting to {}", url);

        *self.shared.status.write() = ConnectionStatus::Connecting;
        emit_state_change(&self.shared);

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

    /// Disconnect from the server and shut down the background loop.
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
        emit_state_change(&self.shared);
        log::debug!("[socket] Disconnected");
        Ok(())
    }

    /// Emit a Socket.IO event to the server.
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
}

impl Default for SocketManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// State-change helpers (used by sibling modules)
// ---------------------------------------------------------------------------

/// Log a state change for observability.
pub(super) fn emit_state_change(shared: &SharedState) {
    let status = *shared.status.read();
    let socket_id = shared.socket_id.read().clone();
    log::debug!("[socket] State changed: {:?}, sid={:?}", status, socket_id);
}

/// Log a server event for observability.
pub(super) fn emit_server_event(_shared: &SharedState, event_name: &str, _data: serde_json::Value) {
    log::debug!("[socket] Server event: {}", event_name);
}
