//! Socket.io state management service
//!
//! This service manages socket connection state and communicates with the frontend.
//! The actual Socket.io client runs in the frontend (JavaScript), while this Rust
//! service:
//! - Tracks connection state
//! - Emits events to the frontend
//! - Manages background execution (socket stays connected when window is hidden)
//!
//! For background execution, the Tauri app keeps running in the system tray,
//! and the frontend's Socket.io connection persists because the WebView is not
//! destroyed when the window is hidden.

use crate::models::socket::{ConnectionStatus, SocketState};
use parking_lot::RwLock;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

/// Events emitted to the frontend
pub mod events {
    pub const SOCKET_CONNECTED: &str = "socket:connected";
    pub const SOCKET_DISCONNECTED: &str = "socket:disconnected";
    pub const SOCKET_ERROR: &str = "socket:error";
    pub const SOCKET_MESSAGE: &str = "socket:message";
    pub const SOCKET_STATE_CHANGED: &str = "socket:state_changed";
    pub const SOCKET_SHOULD_CONNECT: &str = "socket:should_connect";
    pub const SOCKET_SHOULD_DISCONNECT: &str = "socket:should_disconnect";
}

/// Socket state management service
///
/// This service tracks socket connection state and provides an interface
/// for the frontend to report connection status and for the backend to
/// request connection/disconnection.
pub struct SocketService {
    /// Current connection status (reported by frontend)
    status: RwLock<ConnectionStatus>,
    /// Socket ID once connected (reported by frontend)
    socket_id: RwLock<Option<String>>,
    /// Auth token for connection
    auth_token: RwLock<Option<String>>,
    /// Backend URL
    backend_url: RwLock<String>,
    /// App handle for emitting events
    app_handle: RwLock<Option<AppHandle>>,
}

impl SocketService {
    /// Create a new SocketService
    pub fn new() -> Self {
        Self {
            status: RwLock::new(ConnectionStatus::Disconnected),
            socket_id: RwLock::new(None),
            auth_token: RwLock::new(None),
            backend_url: RwLock::new(String::new()),
            app_handle: RwLock::new(None),
        }
    }

    /// Set the app handle for emitting events
    pub fn set_app_handle(&self, handle: AppHandle) {
        *self.app_handle.write() = Some(handle);
    }

    /// Get current connection status
    pub fn get_status(&self) -> ConnectionStatus {
        *self.status.read()
    }

    /// Get current socket state
    pub fn get_state(&self) -> SocketState {
        SocketState {
            status: *self.status.read(),
            socket_id: self.socket_id.read().clone(),
            error: None,
        }
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        *self.status.read() == ConnectionStatus::Connected
    }

    /// Store connection parameters and tell frontend to connect
    pub fn request_connect(&self, backend_url: &str, token: &str) -> Result<(), String> {
        *self.backend_url.write() = backend_url.to_string();
        *self.auth_token.write() = Some(token.to_string());

        // Tell frontend to connect
        if let Some(ref app) = *self.app_handle.read() {
            app.emit(
                events::SOCKET_SHOULD_CONNECT,
                serde_json::json!({
                    "backendUrl": backend_url,
                    "token": token
                }),
            )
            .map_err(|e| format!("Failed to emit connect event: {}", e))?;
        }

        Ok(())
    }

    /// Tell frontend to disconnect
    pub fn request_disconnect(&self) -> Result<(), String> {
        if let Some(ref app) = *self.app_handle.read() {
            app.emit(events::SOCKET_SHOULD_DISCONNECT, ())
                .map_err(|e| format!("Failed to emit disconnect event: {}", e))?;
        }

        Ok(())
    }

    /// Update connection status (called by frontend via command)
    pub fn update_status(&self, status: ConnectionStatus, socket_id: Option<String>) {
        *self.status.write() = status;
        *self.socket_id.write() = socket_id.clone();

        // Emit state change to any listeners
        if let Some(ref app) = *self.app_handle.read() {
            let state = SocketState {
                status,
                socket_id,
                error: None,
            };
            let _ = app.emit(events::SOCKET_STATE_CHANGED, &state);
        }
    }

    /// Report connection (called by frontend)
    pub fn report_connected(&self, socket_id: Option<String>) {
        self.update_status(ConnectionStatus::Connected, socket_id);
    }

    /// Report disconnection (called by frontend)
    pub fn report_disconnected(&self) {
        self.update_status(ConnectionStatus::Disconnected, None);
    }

    /// Report error (called by frontend)
    pub fn report_error(&self, error: &str) {
        *self.status.write() = ConnectionStatus::Error;

        if let Some(ref app) = *self.app_handle.read() {
            let state = SocketState {
                status: ConnectionStatus::Error,
                socket_id: None,
                error: Some(error.to_string()),
            };
            let _ = app.emit(events::SOCKET_STATE_CHANGED, &state);
            let _ = app.emit(events::SOCKET_ERROR, error);
        }
    }

    /// Get stored connection parameters for reconnection
    pub fn get_connection_params(&self) -> Option<(String, String)> {
        let backend_url = self.backend_url.read().clone();
        let token = self.auth_token.read().clone();

        if !backend_url.is_empty() {
            token.map(|t| (backend_url, t))
        } else {
            None
        }
    }

    /// Clear stored credentials
    pub fn clear_credentials(&self) {
        *self.auth_token.write() = None;
        *self.backend_url.write() = String::new();
    }
}

impl Default for SocketService {
    fn default() -> Self {
        Self::new()
    }
}

// Global singleton instance
use once_cell::sync::Lazy;
pub static SOCKET_SERVICE: Lazy<Arc<SocketService>> = Lazy::new(|| Arc::new(SocketService::new()));
