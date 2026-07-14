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

/// Response from the `tunnel:register` ACK callback.
#[derive(Debug, Clone, Deserialize)]
pub struct TunnelRegisterResponse {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    #[serde(rename = "pairingToken")]
    pub pairing_token: String,
    #[serde(rename = "pairingExpiresAt")]
    pub pairing_expires_at: String,
}

/// Payload emitted as `tunnel:connect` to join a channel.
#[derive(Debug, Serialize)]
pub struct TunnelConnectPayload {
    #[serde(rename = "channelId")]
    pub channel_id: String,
    pub role: String, // "core" or "client"
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

/// Emit `tunnel:register` on the shared socket and parse the ACK response.
pub async fn emit_register() -> Result<TunnelRegisterResponse, String> {
    log::debug!("[devices/tunnel] emit_register: sending tunnel:register");
    let mgr = global_socket_manager()
        .ok_or_else(|| "[devices/tunnel] SocketManager not initialized".to_string())?;

    let payload = json!({ "role": "core" });
    let ack = mgr
        .emit_with_ack(
            "tunnel:register",
            payload,
            std::time::Duration::from_secs(10),
        )
        .await
        .map_err(|e| format!("[devices/tunnel] emit tunnel:register failed: {e}"))?;

    serde_json::from_value::<TunnelRegisterResponse>(ack)
        .map_err(|e| format!("[devices/tunnel] parse tunnel:register ack failed: {e}"))
}

/// Emit `tunnel:connect` to start listening on a channel as `role:"core"`.
pub async fn emit_connect(channel_id: &str) -> Result<(), String> {
    log::debug!("[devices/tunnel] emit_connect channel_id={channel_id}");
    let mgr = global_socket_manager()
        .ok_or_else(|| "[devices/tunnel] SocketManager not initialized".to_string())?;

    let payload = build_core_connect_payload(channel_id);

    mgr.emit("tunnel:connect", payload)
        .await
        .map_err(|e| format!("[devices/tunnel] emit tunnel:connect failed: {e}"))
}

fn build_core_connect_payload(channel_id: &str) -> serde_json::Value {
    json!({
        "channelId": channel_id,
        "role": "core",
    })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tunnel_register_response_accepts_backend_ack_shape_without_session_token() {
        let response: TunnelRegisterResponse = serde_json::from_value(json!({
            "channelId": "ch_123",
            "pairingToken": "pt_123",
            "pairingExpiresAt": "2026-06-30T15:00:00Z"
        }))
        .expect("backend register ack shape should parse");

        assert_eq!(response.channel_id, "ch_123");
        assert_eq!(response.pairing_token, "pt_123");
        assert_eq!(response.pairing_expires_at, "2026-06-30T15:00:00Z");
    }

    #[test]
    fn build_core_connect_payload_omits_session_token_for_core_role() {
        let payload = build_core_connect_payload("ch_123");

        assert_eq!(payload["channelId"], "ch_123");
        assert_eq!(payload["role"], "core");
        assert!(payload.get("sessionToken").is_none());
        assert!(payload.get("pairingToken").is_none());
    }
}
