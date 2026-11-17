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
    /// TinySecurity owner id used for module-side revoke and recovery.
    /// Missing on legacy rows until import reconciliation succeeds.
    #[serde(skip)]
    pub(crate) pairing_id: Option<String>,
    /// Legacy encrypted core identity retained only as an import source for
    /// pre-TinySecurity device rows. New pairings keep identity ciphertext in
    /// TinySecurity's own persistence record.
    #[serde(skip)]
    pub(crate) core_private_key_encrypted: Option<String>,
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
    /// Host-minted socket nonce bound by TinySecurity Begin and the first frame.
    #[serde(skip)]
    pub(crate) connection_nonce: String,
    /// Base64url-encoded X25519 public key returned by TinySecurity.
    pub core_pubkey: String,
    /// TinySecurity owner receipt consumed by the next authenticated handshake.
    #[serde(skip)]
    pub(crate) pairing_receipt: String,
    /// User supplied label retained until the module confirms enrollment.
    #[serde(skip)]
    pub(crate) label: String,
    /// Optional LAN URL for the direct HTTP fast path.
    pub rpc_url: Option<String>,
    /// ISO 8601 timestamp when the pairing token expires.
    pub expires_at: String,
    /// The agent that started the pairing (`CoreContext::session_agent`),
    /// when it was acting for one: the paired device is stored in, and its
    /// tunnel frames are handled as, that agent (`devices::owner`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
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

/// Summary of an explicit one-time legacy device import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportLegacyResponse {
    pub imported: usize,
}
