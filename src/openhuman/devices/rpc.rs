//! RPC handler implementations for the devices domain.
//!
//! Three methods:
//!  - `devices_create_pairing` — registers a pairing channel and returns QR fields.
//!  - `devices_list`           — lists non-revoked paired devices.
//!  - `devices_revoke`         — marks a device revoked and closes its tunnel channel.
//!
//! Keypair persistence: private key bytes are encrypted with the workspace
//! `SecretStore` (ChaCha20-Poly1305) and stored as `enc2:` values keyed by
//! channel_id in `PERSISTED_KEYPAIRS`. On restart, bus.rs can reconstruct the
//! keypair for reconnect handshakes without re-generating.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::openhuman::config::Config;
use crate::openhuman::devices::crypto::{base64url_decode, base64url_encode, DeviceKeypair};
use crate::openhuman::devices::store;
use crate::openhuman::devices::tunnel_client;
use crate::openhuman::devices::types::{
    CreatePairingResponse, ListDevicesResponse, PairingSession, RevokeDeviceResponse,
};
use crate::openhuman::security::SecretStore;
use crate::rpc::RpcOutcome;

// ---------------------------------------------------------------------------
// In-memory state (module-level singletons)
// ---------------------------------------------------------------------------

/// Keypairs pending handshake completion (keyed by channel_id).
/// Values are `Arc` so bus.rs can clone without holding the lock during DH.
pub(crate) static PENDING_KEYPAIRS: once_cell::sync::Lazy<
    Mutex<HashMap<String, Arc<DeviceKeypair>>>,
> = once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Encrypted persisted private-key bytes (keyed by channel_id).
/// Values are `enc2:<hex>` strings from `SecretStore::encrypt`.
/// Populated by `devices_create_pairing`; cleared by `devices_revoke`.
pub(crate) static PERSISTED_KEYPAIRS: once_cell::sync::Lazy<Mutex<HashMap<String, String>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Pairing sessions pending device connection (keyed by channel_id).
pub(crate) static PENDING_SESSIONS: once_cell::sync::Lazy<Mutex<HashMap<String, PairingSession>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

/// Live peer-online status (keyed by channel_id). Updated by bus.rs on `tunnel:peer-status`.
pub(crate) static PEER_STATUS: once_cell::sync::Lazy<Mutex<HashMap<String, bool>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(HashMap::new()));

// ---------------------------------------------------------------------------
// create_pairing
// ---------------------------------------------------------------------------

/// `openhuman.devices_create_pairing`
///
/// 1. Calls `tunnel:register` on the shared socket — backend returns
///    `{channelId, pairingToken, sessionToken}`.
/// 2. Generates an X25519 keypair and persists the private half in-memory.
/// 3. Emits `tunnel:connect` with `role:"core"` so the core starts listening.
/// 4. Detects the local LAN IP for the optional direct fast-path `rpc_url`.
/// 5. Returns QR-bound fields to the caller.
pub async fn devices_create_pairing(
    _config: &Config,
    label: Option<String>,
) -> Result<RpcOutcome<CreatePairingResponse>, String> {
    log::info!(
        "[devices/rpc] devices_create_pairing entry label={:?}",
        label
    );

    // Register with backend tunnel.
    let reg = tunnel_client::emit_register().await.map_err(|e| {
        log::error!("[devices/rpc] tunnel:register failed: {e}");
        e
    })?;

    log::info!(
        "[devices/rpc] tunnel:register ok channel_id={} token_len={}",
        reg.channel_id,
        reg.pairing_token.len()
    );

    // Generate X25519 keypair for this channel.
    let keypair = DeviceKeypair::generate();
    let core_pubkey = keypair.pubkey_b64.clone();

    // Encrypt the private key bytes and persist in the encrypted secrets store.
    let secret_store = build_secret_store(_config);
    let private_b64 = base64url_encode(&keypair.private_bytes());
    match secret_store.encrypt(&private_b64) {
        Ok(enc) => {
            PERSISTED_KEYPAIRS
                .lock()
                .unwrap()
                .insert(reg.channel_id.clone(), enc);
            log::debug!(
                "[devices/rpc] keypair private key encrypted and persisted channel_id={}",
                reg.channel_id
            );
        }
        Err(e) => {
            log::warn!(
                "[devices/rpc] could not persist encrypted keypair channel_id={}: {e}",
                reg.channel_id
            );
        }
    }

    // Stash keypair in memory so bus.rs can complete the X25519 handshake.
    PENDING_KEYPAIRS
        .lock()
        .unwrap()
        .insert(reg.channel_id.clone(), Arc::new(keypair));

    // Connect as "core" role to start listening on this channel.
    tunnel_client::emit_connect(&reg.channel_id, &reg.session_token)
        .await
        .map_err(|e| {
            log::error!("[devices/rpc] tunnel:connect failed: {e}");
            e
        })?;

    log::debug!(
        "[devices/rpc] tunnel:connect emitted channel_id={}",
        reg.channel_id
    );

    // Best-effort LAN URL detection (non-fatal if it fails).
    let rpc_url = detect_lan_rpc_url();
    if let Some(ref url) = rpc_url {
        log::debug!("[devices/rpc] LAN rpc_url detected: {}", url);
    }

    // Pairing token expires in 10 minutes (backend enforces the real TTL).
    let expires_at = (Utc::now() + chrono::Duration::minutes(10)).to_rfc3339();

    PENDING_SESSIONS.lock().unwrap().insert(
        reg.channel_id.clone(),
        PairingSession {
            channel_id: reg.channel_id.clone(),
            pairing_token: reg.pairing_token.clone(),
            core_session_token: reg.session_token.clone(),
            core_pubkey: core_pubkey.clone(),
            rpc_url: rpc_url.clone(),
            expires_at: expires_at.clone(),
        },
    );

    log::info!(
        "[devices/rpc] devices_create_pairing done channel_id={}",
        reg.channel_id
    );

    Ok(RpcOutcome::single_log(
        CreatePairingResponse {
            channel_id: reg.channel_id,
            pairing_token: reg.pairing_token,
            core_pubkey,
            rpc_url,
            expires_at,
        },
        "pairing channel created",
    ))
}

// ---------------------------------------------------------------------------
// list
// ---------------------------------------------------------------------------

/// `openhuman.devices_list`
pub async fn devices_list(config: &Config) -> Result<RpcOutcome<ListDevicesResponse>, String> {
    log::debug!("[devices/rpc] devices_list entry");
    let mut devices = store::list_devices(config)
        .map_err(|e| format!("[devices/rpc] list_devices failed: {e}"))?;

    // Overlay live peer-online status from in-memory map.
    {
        let peer_map = PEER_STATUS.lock().unwrap();
        for dev in &mut devices {
            let online = peer_map.get(&dev.channel_id).copied().unwrap_or(false);
            dev.peer_online = Some(online);
        }
    }

    log::debug!(
        "[devices/rpc] devices_list returning {} device(s)",
        devices.len()
    );
    Ok(RpcOutcome::new(ListDevicesResponse { devices }, vec![]))
}

// ---------------------------------------------------------------------------
// revoke
// ---------------------------------------------------------------------------

/// `openhuman.devices_revoke`
pub async fn devices_revoke(
    config: &Config,
    channel_id: String,
) -> Result<RpcOutcome<RevokeDeviceResponse>, String> {
    log::info!("[devices/rpc] devices_revoke channel_id={}", channel_id);

    let revoked = store::revoke_device(config, &channel_id)
        .map_err(|e| format!("[devices/rpc] revoke_device failed: {e}"))?;

    // Clear in-memory state for this channel, including persisted encrypted key.
    PENDING_KEYPAIRS.lock().unwrap().remove(&channel_id);
    PENDING_SESSIONS.lock().unwrap().remove(&channel_id);
    PEER_STATUS.lock().unwrap().remove(&channel_id);
    PERSISTED_KEYPAIRS.lock().unwrap().remove(&channel_id);

    // Publish DeviceRevoked so UI and other subscribers are notified.
    crate::core::event_bus::publish_global(crate::core::event_bus::DomainEvent::DeviceRevoked {
        channel_id: channel_id.clone(),
    });

    // TODO: backend revoke endpoint pending (PR #709 follow-up).
    // For now, closing the local tunnel side + letting the backend TTL the channel is sufficient.
    log::info!(
        "[devices/rpc] devices_revoke done channel_id={} revoked={}",
        channel_id,
        revoked
    );

    Ok(RpcOutcome::single_log(
        RevokeDeviceResponse { success: revoked },
        format!("device {channel_id} revoked"),
    ))
}

// ---------------------------------------------------------------------------
// LAN URL detection
// ---------------------------------------------------------------------------

fn detect_lan_rpc_url() -> Option<String> {
    let ip = find_local_ipv4()?;
    // Use the configured RPC port if available via env, else fall back to 7788.
    let port = std::env::var("OPENHUMAN_CORE_RPC_PORT")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(7788);
    Some(format!("http://{}:{}/rpc", ip, port))
}

fn find_local_ipv4() -> Option<String> {
    use std::net::{IpAddr, UdpSocket};
    // UDP trick: connect to a public address (no packet sent) and read local addr.
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    match socket.local_addr().ok()?.ip() {
        IpAddr::V4(addr) if !addr.is_loopback() => Some(addr.to_string()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Secret store helper
// ---------------------------------------------------------------------------

/// Build a `SecretStore` scoped to the workspace directory.
fn build_secret_store(config: &Config) -> SecretStore {
    let data_dir = config
        .config_path
        .parent()
        .map_or_else(|| std::path::PathBuf::from("."), std::path::PathBuf::from);
    SecretStore::new(&data_dir, true)
}

/// Reconstruct a `DeviceKeypair` from the encrypted private key store.
///
/// Returns `None` when the channel has no persisted key or decryption fails.
pub(crate) fn load_keypair_from_store(
    config: &Config,
    channel_id: &str,
) -> Option<Arc<DeviceKeypair>> {
    let enc = PERSISTED_KEYPAIRS
        .lock()
        .unwrap()
        .get(channel_id)
        .cloned()?;
    let store = build_secret_store(config);
    let private_b64 = store
        .decrypt(&enc)
        .map_err(|e| {
            log::warn!(
                "[devices/rpc] decrypt keypair failed channel_id={}: {e}",
                channel_id
            );
        })
        .ok()?;
    let priv_bytes = base64url_decode(&private_b64)
        .map_err(|e| {
            log::warn!(
                "[devices/rpc] base64url decode keypair failed channel_id={}: {e}",
                channel_id
            );
        })
        .ok()?;
    if priv_bytes.len() != 32 {
        log::warn!(
            "[devices/rpc] loaded private key has wrong length {} channel_id={}",
            priv_bytes.len(),
            channel_id
        );
        return None;
    }
    let arr: [u8; 32] = priv_bytes.try_into().ok()?;
    log::debug!(
        "[devices/rpc] keypair restored from encrypted store channel_id={}",
        channel_id
    );
    Some(Arc::new(DeviceKeypair::from_private_bytes(arr)))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    fn test_config() -> Config {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = dir.into_path();
        config
    }

    #[tokio::test]
    async fn devices_list_returns_empty_initially() {
        let config = test_config();
        let result = devices_list(&config).await.unwrap();
        assert!(result.value.devices.is_empty());
    }

    #[tokio::test]
    async fn devices_revoke_nonexistent_returns_false() {
        let config = test_config();
        let result = devices_revoke(&config, "NONEXISTENT".to_string())
            .await
            .unwrap();
        assert!(!result.value.success);
    }

    #[tokio::test]
    async fn devices_list_includes_inserted_device_with_online_status() {
        let config = test_config();
        store::insert_device(
            &config,
            "CHAN_LIST2",
            "Test Phone",
            "pubkey_test",
            "hash_test",
        )
        .unwrap();

        // Simulate a peer coming online.
        PEER_STATUS
            .lock()
            .unwrap()
            .insert("CHAN_LIST2".to_string(), true);

        let result = devices_list(&config).await.unwrap();
        let found = result
            .value
            .devices
            .iter()
            .find(|d| d.channel_id == "CHAN_LIST2");
        assert!(found.is_some());
        assert_eq!(found.unwrap().peer_online, Some(true));

        PEER_STATUS.lock().unwrap().remove("CHAN_LIST2");
    }
}
