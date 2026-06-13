//! Domain types for the devices (mobile pairing) domain.

use serde::{Deserialize, Serialize};

/// A successfully paired mobile device persisted in SQLite.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedDevice {
    /// 128-bit base32 channel identifier assigned by the backend tunnel.
    pub channel_id: String,
    /// Human-readable label, e.g. "iPhone 15".
    pub label: String,
    /// Base64url-encoded X25519 public key of the device.
    pub device_pubkey: String,
    /// ISO 8601 creation timestamp.
    pub created_at: String,
    /// ISO 8601 timestamp of most recent tunnel activity, if any.
    pub last_seen_at: Option<String>,
    /// Derived from `tunnel:peer-status`; not persisted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peer_online: Option<bool>,
    /// True once `devices_revoke` has been called.
    pub revoked: bool,
}

/// Short-lived pairing session created by `devices_create_pairing`.
///
/// Lives in memory (in a `DashMap`) with a TTL cleanup task. Never written to
/// SQLite — the backend already enforces the single-use / TTL semantics on the
/// pairing token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingSession {
    /// 128-bit base32 channel identifier from `tunnel:register`.
    pub channel_id: String,
    /// Base64url pairing token (single-use, TTL'd, hashed at rest on backend).
    pub pairing_token: String,
    /// Core's reconnect credential for this channel (hashed at rest in SQLite).
    pub core_session_token: String,
    /// Base64url-encoded X25519 public key generated for this pairing.
    pub core_pubkey: String,
    /// Optional LAN URL for the direct HTTP fast path.
    pub rpc_url: Option<String>,
    /// ISO 8601 timestamp when the pairing token expires.
    pub expires_at: String,
}

/// Response payload for `devices_create_pairing`.
#[derive(Debug, Serialize, Deserialize)]
pub struct CreatePairingResponse {
    pub channel_id: String,
    pub pairing_token: String,
    pub core_pubkey: String,
    pub rpc_url: Option<String>,
    pub expires_at: String,
}

/// Response payload for `devices_list`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ListDevicesResponse {
    pub devices: Vec<PairedDevice>,
}

/// Response payload for `devices_revoke`.
#[derive(Debug, Serialize, Deserialize)]
pub struct RevokeDeviceResponse {
    pub success: bool,
}
