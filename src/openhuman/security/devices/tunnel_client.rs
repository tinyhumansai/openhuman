//! Tunnel client for the device pairing domain.
//!
//! Reuses the existing `SocketManager` (global singleton) to emit and receive
//! `tunnel:*` Socket.IO events without opening a second WebSocket connection to
//! the backend. Incoming `tunnel:peer-status` and `tunnel:frame` events arrive
//! via the event bus (published by `socket::event_handlers` after this module
//! adds them to the dispatch table) and are handled by `devices::bus`.
//!
//! Frame cap: 64 KB. Rate limit: callers are expected to stay ≤ 100 frames/s.

use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::openhuman::platform::socket::global_socket_manager;

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
    /// Backend has been observed sending this as either an ISO 8601 string
    /// or an epoch-millisecond integer — normalize both to an ISO 8601
    /// string so every downstream consumer (QR `exp` field, frontend TTL
    /// checks) keeps seeing the contract's documented shape.
    #[serde(rename = "pairingExpiresAt", deserialize_with = "deserialize_expires_at")]
    pub pairing_expires_at: String,
}

fn deserialize_expires_at<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrEpochMs {
        Str(String),
        EpochMs(i64),
    }

    match StringOrEpochMs::deserialize(deserializer)? {
        StringOrEpochMs::Str(s) => Ok(s),
        StringOrEpochMs::EpochMs(ms) => Utc
            .timestamp_millis_opt(ms)
            .single()
            .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true))
            .ok_or_else(|| {
                serde::de::Error::custom(format!(
                    "pairingExpiresAt: epoch-ms {ms} is out of range"
                ))
            }),
    }
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

    // Logged at warn (not debug) so it shows up under the default RUST_LOG=info
    // — this backend's ack shape has been observed changing between attempts
    // (wrong-typed pairingExpiresAt, then a missing channelId entirely), so
    // seeing the exact raw payload is the fastest way to tell "flaky backend"
    // from "our struct is wrong."
    log::warn!("[devices/tunnel] raw tunnel:register ack: {ack}");

    // The backend acks a rejected registration with an error object shaped
    // like `{"error": "<code>", "ok": false}` rather than an HTTP-level
    // error — e.g. `tunnel_limit_reached` when too many pending/unreleased
    // channels are already open for this account (there is no backend
    // "release early" endpoint yet; pending channels only clear via their
    // ~10 minute TTL, see devices/README.md). Detecting this shape first
    // turns a confusing "missing field `channelId`" parse failure into the
    // real reason.
    if let Some(err) = register_ack_error(&ack) {
        return Err(err);
    }

    serde_json::from_value::<TunnelRegisterResponse>(ack)
        .map_err(|e| format!("[devices/tunnel] parse tunnel:register ack failed: {e}"))
}

/// Recognizes the backend's `{"error": "<code>", "ok": false}` rejection
/// shape for `tunnel:register` and turns it into a clear message. Returns
/// `None` for anything else (including a genuine success payload), leaving
/// that to the normal `TunnelRegisterResponse` parse.
fn register_ack_error(ack: &serde_json::Value) -> Option<String> {
    if ack.get("ok").and_then(|v| v.as_bool()) != Some(false) {
        return None;
    }
    let code = ack.get("error").and_then(|v| v.as_str()).unwrap_or("unknown_error");
    Some(match code {
        "tunnel_limit_reached" => {
            "[devices/tunnel] tunnel:register rejected: too many pending device pairings \
             are already open for this account. Wait a few minutes for old ones to expire \
             (~10 min TTL) and try again."
                .to_string()
        }
        other => format!("[devices/tunnel] tunnel:register rejected: {other}"),
    })
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
    fn register_ack_error_recognizes_tunnel_limit_reached() {
        let ack = json!({"error": "tunnel_limit_reached", "ok": false});
        let err = register_ack_error(&ack).expect("should recognize the error shape");
        assert!(err.contains("too many pending device pairings"), "got: {err}");
    }

    #[test]
    fn register_ack_error_passes_through_unknown_error_codes() {
        let ack = json!({"error": "something_else", "ok": false});
        let err = register_ack_error(&ack).expect("should recognize any ok:false shape");
        assert!(err.contains("something_else"), "got: {err}");
    }

    #[test]
    fn register_ack_error_ignores_success_shapes() {
        let ack = json!({
            "channelId": "ch_123",
            "pairingToken": "pt_123",
            "pairingExpiresAt": "2026-06-30T15:00:00Z"
        });
        assert_eq!(register_ack_error(&ack), None);
    }

    #[test]
    fn tunnel_register_response_accepts_epoch_ms_pairing_expires_at() {
        // Observed live from api.tinyhumans.ai: pairingExpiresAt sent as an
        // integer epoch-ms timestamp rather than the documented ISO 8601
        // string. Must normalize to a string, not fail to parse.
        let response: TunnelRegisterResponse = serde_json::from_value(json!({
            "channelId": "ch_123",
            "pairingToken": "pt_123",
            "pairingExpiresAt": 1787784497036i64
        }))
        .expect("epoch-ms pairingExpiresAt should parse");

        assert_eq!(response.pairing_expires_at, "2026-08-26T22:48:17.036Z");
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
