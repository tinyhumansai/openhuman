//! Tunnel client for the device pairing domain.
//!
//! Reuses the existing `SocketManager` (global singleton) to emit and receive
//! `tunnel:*` Socket.IO events without opening a second WebSocket connection to
//! the backend. Incoming `tunnel:peer-status` and `tunnel:frame` events arrive
//! via the event bus (published by `socket::event_handlers` after this module
//! adds them to the dispatch table) and are handled by `devices::bus`.
//!
//! Frame cap: 64 KB. Rate limit: callers are expected to stay ≤ 100 frames/s.

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::openhuman::socket::global_socket_manager;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// Payload emitted as `tunnel:register` to the backend.
#[derive(Debug, Serialize)]
pub struct TunnelRegisterPayload {
    pub role: String, // always "core"
}

/// Response from `tunnel:register` emitted back by the backend.
#[derive(Debug, Clone, Deserialize)]
pub struct TunnelRegisterResponse {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(rename = "pairingToken")]
    pub pairing_token: String,
    #[serde(rename = "sessionToken")]
    pub session_token: String,
}

/// Payload emitted as `tunnel:connect` to join a channel.
#[derive(Debug, Serialize)]
pub struct TunnelConnectPayload {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub role: String, // "core" or "client"
    #[serde(rename = "sessionToken", skip_serializing_if = "Option::is_none")]
    pub session_token: Option<String>,
    #[serde(rename = "pairingToken", skip_serializing_if = "Option::is_none")]
    pub pairing_token: Option<String>,
}

/// Inbound `tunnel:peer-status` event payload.
#[derive(Debug, Clone, Deserialize)]
pub struct TunnelPeerStatus {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub online: bool,
}

/// Inbound `tunnel:frame` event payload.
#[derive(Debug, Clone, Deserialize)]
pub struct TunnelFrame {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    /// Base64url-encoded encrypted frame bytes.
    pub payload: String,
}

/// Outbound `tunnel:frame` emit payload.
#[derive(Debug, Serialize)]
struct TunnelFrameEmit<'a> {
    #[serde(rename = "channelId")]
    channel_id: &'a str,
    payload: &'a str,
}

// ---------------------------------------------------------------------------
// Tunnel operations
// ---------------------------------------------------------------------------

/// Emit `tunnel:register` on the shared socket and parse the response.
///
/// The backend returns `{channelId, pairingToken, sessionToken}` via the
/// same socket in a `tunnel:registered` ack. Since the existing `SocketManager`
/// does not support request/response acks over the raw WebSocket, we use
/// a one-shot `tokio::sync::oneshot` channel registered in a global pending-ack
/// map and resolved by `devices::bus` when the `tunnel:registered` event arrives.
///
/// For v1 this is simplified: we emit the registration event and expect the
/// caller (rpc.rs) to await the response via the in-process ack mechanism.
pub async fn emit_register() -> Result<TunnelRegisterResponse, String> {
    log::debug!("[devices/tunnel] emit_register: sending tunnel:register");
    let mgr = global_socket_manager()
        .ok_or_else(|| "[devices/tunnel] SocketManager not initialized".to_string())?;

    let payload = json!({ "role": "core" });

    // Register a pending ack before emitting to avoid a race.
    let rx = PENDING_REGISTER.register_pending();

    mgr.emit("tunnel:register", payload)
        .await
        .map_err(|e| format!("[devices/tunnel] emit tunnel:register failed: {e}"))?;

    log::debug!("[devices/tunnel] tunnel:register emitted, awaiting response");

    // Wait up to 10 s for the backend ack.
    tokio::time::timeout(std::time::Duration::from_secs(10), rx)
        .await
        .map_err(|_| "[devices/tunnel] timeout waiting for tunnel:registered".to_string())?
        .map_err(|_| "[devices/tunnel] ack channel dropped".to_string())
}

/// Emit `tunnel:connect` to start listening on a channel as `role:"core"`.
pub async fn emit_connect(channel_id: &str, session_token: &str) -> Result<(), String> {
    log::debug!(
        "[devices/tunnel] emit_connect channel_id={} token_len={}",
        channel_id,
        session_token.len()
    );
    let mgr = global_socket_manager()
        .ok_or_else(|| "[devices/tunnel] SocketManager not initialized".to_string())?;

    let payload = json!({
        "channelId": channel_id,
        "role": "core",
        "sessionToken": session_token,
    });

    mgr.emit("tunnel:connect", payload)
        .await
        .map_err(|e| format!("[devices/tunnel] emit tunnel:connect failed: {e}"))
}

/// Emit a `tunnel:frame` carrying an encrypted payload for the peer.
///
/// `payload_b64` is the base64url-encoded sealed frame from `TunnelCipher::seal`.
pub async fn emit_frame(channel_id: &str, payload_b64: &str) -> Result<(), String> {
    if payload_b64.len() > 64 * 1024 {
        return Err(format!(
            "[devices/tunnel] frame too large: {} bytes (max 64 KB)",
            payload_b64.len()
        ));
    }
    let mgr = global_socket_manager()
        .ok_or_else(|| "[devices/tunnel] SocketManager not initialized".to_string())?;

    let payload = json!({
        "channelId": channel_id,
        "payload": payload_b64,
    });

    mgr.emit("tunnel:frame", payload)
        .await
        .map_err(|e| format!("[devices/tunnel] emit tunnel:frame failed: {e}"))
}

/// Resolve a pending `tunnel:register` ack when the backend responds.
///
/// Called by `socket::event_handlers` when it receives `tunnel:registered`.
pub fn resolve_register_ack(response: TunnelRegisterResponse) {
    log::debug!(
        "[devices/tunnel] resolving tunnel:registered ack channel_id={}",
        response.channel_id
    );
    PENDING_REGISTER.resolve(response);
}

// ---------------------------------------------------------------------------
// One-shot ack registry for tunnel:register
// ---------------------------------------------------------------------------

use std::sync::Mutex;
use tokio::sync::oneshot;

struct PendingRegisterAck {
    tx: Mutex<Option<oneshot::Sender<TunnelRegisterResponse>>>,
}

impl PendingRegisterAck {
    const fn new() -> Self {
        Self {
            tx: Mutex::new(None),
        }
    }

    fn register_pending(&self) -> oneshot::Receiver<TunnelRegisterResponse> {
        let (tx, rx) = oneshot::channel();
        *self.tx.lock().unwrap() = Some(tx);
        rx
    }

    fn resolve(&self, response: TunnelRegisterResponse) {
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(response);
        }
    }
}

static PENDING_REGISTER: PendingRegisterAck = PendingRegisterAck::new();
