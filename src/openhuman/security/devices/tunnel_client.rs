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
///
/// The backend sends **camelCase**: backend PR #709 introduced
/// `TunnelRegisterAck { channelId, pairingToken, pairingExpiresAt: number }`
/// (`socketHandlers/tunnel/types.ts`) and `main` still emits exactly that. The
/// `alias` entries are additive compatibility for a snake_case shape nothing
/// currently sends — kept so a future rename needs no coordinated deploy, not
/// because a rename has happened.
///
/// An earlier version of this comment had the history backwards, describing
/// #709 as the move *to* snake_case. It is recorded here because the two real
/// defects in this path — a numeric `pairingExpiresAt` decoded as a string, and
/// a `{ ok: false }` refusal parsed as the success shape — were both missed
/// while the field names were the suspect.
#[derive(Debug, Clone, Deserialize)]
pub struct TunnelRegisterResponse {
    #[serde(rename = "channelId", alias = "channel_id")]
    pub channel_id: String,
    #[serde(rename = "pairingToken", alias = "pairing_token")]
    pub pairing_token: String,
    /// Deserialized through [`expires_at_from_string_or_epoch_millis`]: the
    /// backend sends this as a JSON **number** (`pairingExpiresAt:
    /// r.pairingExpiresAt.getTime()` in `socketHandlers/tunnel/handler.ts`),
    /// while the field — and `PairingSession::expires_at` downstream — is
    /// documented and consumed as an ISO 8601 string.
    #[serde(
        rename = "pairingExpiresAt",
        alias = "pairing_expires_at",
        deserialize_with = "expires_at_from_string_or_epoch_millis"
    )]
    pub pairing_expires_at: String,
}

/// Floor for a plausible epoch-milliseconds `pairingExpiresAt`: 2020-01-01Z.
///
/// Anything below it is far likelier to be seconds than a real expiry, and
/// seconds decode silently into 1970 rather than failing.
const MIN_PLAUSIBLE_EPOCH_MILLIS: i64 = 1_577_836_800_000;

/// Accept `pairingExpiresAt` as either an ISO 8601 string or epoch
/// milliseconds, always yielding the ISO 8601 string the rest of the domain
/// expects.
///
/// The backend's `TunnelRegisterAck` types this field as `number` and fills it
/// with `Date.getTime()`, so a plain `String` field cannot deserialize a
/// successful ACK at all — it fails with `invalid type: integer, expected a
/// string`. Widening to `i64` instead would push the epoch value into
/// `PairingSession::expires_at` and `CreatePairingResponse::expires_at`, both
/// documented as "ISO 8601 timestamp" and handed to the paired device, so the
/// conversion happens here where the wire shape is known.
fn expires_at_from_string_or_epoch_millis<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error as _;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Text(String),
        EpochMillis(i64),
    }

    match Raw::deserialize(deserializer)? {
        Raw::Text(text) => Ok(text),
        Raw::EpochMillis(millis) => {
            // Reject a value that is not plausibly milliseconds. `getTime()`
            // returns millis, but a backend that ever switched to seconds would
            // hand over ~1.79e9, and `from_timestamp_millis` accepts that
            // happily as 1970-01-21 — a pairing that is already expired before
            // the QR is drawn, reported to the user as "expired" with nothing
            // in the log to say why. Failing loudly here keeps the silent
            // version of this PR's own bug from being reintroduced by a unit
            // change upstream.
            if millis < MIN_PLAUSIBLE_EPOCH_MILLIS {
                return Err(D::Error::custom(format!(
                    "pairingExpiresAt={millis} is too small to be epoch milliseconds \
                     (expected > {MIN_PLAUSIBLE_EPOCH_MILLIS}); a seconds-valued expiry \
                     would silently decode as 1970"
                )));
            }
            chrono::DateTime::from_timestamp_millis(millis)
                .map(|dt| dt.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
                .ok_or_else(|| D::Error::custom(format!("pairingExpiresAt out of range: {millis}")))
        }
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

    parse_register_ack(ack)
}

/// Turn a `tunnel:register` ACK into a response or an error message.
///
/// Split out of [`emit_register`] so the whole decision — failure envelope
/// first, success shape second — is reachable without a live `SocketManager`.
/// Testing only the [`backend_ack_error`] predicate would not have caught the
/// envelope check being dropped from the call path, which is the regression
/// that matters.
pub(crate) fn parse_register_ack(ack: serde_json::Value) -> Result<TunnelRegisterResponse, String> {
    // A failed register is answered with `{ ok: false, error: "..." }`
    // (`safeAck<TunnelRegisterAck>(ack, { ok: false, error: publicError(err) })`
    // in the backend handler), which carries no `channelId`. Parsing that as
    // the success shape reports `missing field 'channelId'` and throws away the
    // reason the backend gave — which is exactly what #5871 was: a register
    // that failed server-side, surfaced to the user as a client parse error.
    if let Some(error) = backend_ack_error(&ack) {
        log::error!("[devices/tunnel] tunnel:register refused by backend: {error}");
        return Err(format!("[devices/tunnel] tunnel:register failed: {error}"));
    }

    // Describe the shape before `from_value` consumes the value, so the success
    // path pays no clone and the failure path can still say what arrived.
    let shape = describe_ack_shape(&ack);
    serde_json::from_value::<TunnelRegisterResponse>(ack).map_err(|e| {
        log::error!("[devices/tunnel] parse tunnel:register ack failed: {e}; ack was a {shape}");
        format!("[devices/tunnel] parse tunnel:register ack failed: {e}")
    })
}

/// The backend's error message when an ACK carries the `{ ok: false, error }`
/// failure envelope, or `None` for anything else.
///
/// `ok` is only ever present on the failure path — a successful
/// `TunnelRegisterAck` has no such field — so an ACK carrying `ok: false` is
/// unambiguously a refusal and its `error` is the only useful thing in it.
pub(crate) fn backend_ack_error(ack: &serde_json::Value) -> Option<String> {
    let object = ack.as_object()?;
    // Absent `ok` is the success shape. Anything else is a refusal — including
    // a present-but-not-`true` value such as the string `"false"` or `0`. An
    // earlier version asked `as_bool()?`, which yields `None` for those and so
    // fell through to `missing field 'channelId'`: the exact bug this function
    // exists to prevent, reintroduced for a backend that answers sloppily.
    if object.get("ok")?.as_bool() == Some(true) {
        return None;
    }
    // A non-string `error` (say `{ code, message }`) is rendered rather than
    // discarded — throwing away the backend's explanation is the failure mode
    // being fixed, and it does not become acceptable because the shape
    // surprised us.
    Some(match object.get("error") {
        None | Some(serde_json::Value::Null) => "unspecified error".to_string(),
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
    })
}

/// A short description of an ACK's shape, for the parse-failure log.
///
/// `as_object()` is `None` for an array, a string or `null` — precisely the
/// cases where "what actually arrived?" is the open question, and precisely
/// where an earlier version logged the unhelpful `ack fields = None`.
fn describe_ack_shape(ack: &serde_json::Value) -> String {
    match ack {
        serde_json::Value::Object(map) => {
            format!("object with fields {:?}", map.keys().collect::<Vec<_>>())
        }
        serde_json::Value::Array(items) => format!("array of {} element(s)", items.len()),
        serde_json::Value::String(_) => "string".to_string(),
        serde_json::Value::Number(_) => "number".to_string(),
        serde_json::Value::Bool(_) => "bool".to_string(),
        serde_json::Value::Null => "null".to_string(),
    }
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
#[path = "tunnel_client_tests.rs"]
mod tests;
