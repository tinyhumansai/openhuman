//! Event bus handlers for the devices domain.
//!
//! Subscribes to `tunnel:peer-status` and `tunnel:frame` events published by
//! `socket::event_handlers` and drives:
//! - Updating `PEER_STATUS` in `rpc.rs`.
//! - Completing the X25519 handshake when the device sends its pubkey.
//! - Persisting the `PairedDevice` record after a successful handshake.
//! - Publishing `DomainEvent::DevicePaired / DevicePeerOnline / DevicePeerOffline`.
//! - Resolving `tunnel:registered` acks for `tunnel_client`.

use std::sync::{Arc, OnceLock};

use crate::core::event_bus::{publish_global, DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::devices::crypto::{
    base64url_decode, base64url_encode, derive_session_keys, TunnelCipher, TunnelRole,
};
use crate::openhuman::devices::rpc::{
    ACTIVE_CIPHERS, PEER_STATUS, PENDING_KEYPAIRS, PENDING_SESSIONS,
};
use crate::openhuman::devices::store;
use crate::openhuman::devices::tunnel_client::{
    emit_frame, resolve_register_ack, TunnelRegisterResponse,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use x25519_dalek::{PublicKey, StaticSecret};

static DEVICE_TUNNEL_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the device tunnel subscriber on the global event bus.
/// Idempotent — subsequent calls are no-ops.
pub fn register_device_tunnel_subscriber() {
    if DEVICE_TUNNEL_HANDLE.get().is_some() {
        return;
    }
    match crate::core::event_bus::subscribe_global(Arc::new(DeviceTunnelSubscriber::new())) {
        Some(handle) => {
            let _ = DEVICE_TUNNEL_HANDLE.set(handle);
            log::info!("[devices/bus] DeviceTunnelSubscriber registered");
        }
        None => {
            log::warn!(
                "[devices/bus] failed to register DeviceTunnelSubscriber — bus not initialized"
            );
        }
    }
}

/// Subscribes to device tunnel events from the event bus.
pub struct DeviceTunnelSubscriber;

impl DeviceTunnelSubscriber {
    pub fn new() -> Self {
        Self
    }
}

impl Default for DeviceTunnelSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EventHandler for DeviceTunnelSubscriber {
    fn name(&self) -> &str {
        "device::tunnel"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["device"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::DevicePeerOnline { channel_id } => {
                handle_peer_online(channel_id).await;
            }
            DomainEvent::DevicePeerOffline { channel_id } => {
                handle_peer_offline(channel_id);
            }
            DomainEvent::DeviceTunnelFrame {
                channel_id,
                payload_b64,
            } => {
                handle_tunnel_frame(channel_id, payload_b64).await;
            }
            DomainEvent::DeviceTunnelRegistered {
                channel_id,
                pairing_token,
                session_token,
            } => {
                handle_registered(channel_id, pairing_token, session_token);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct HandshakePayload {
    device_pubkey: String,
    client_ephemeral_pubkey: Option<String>,
}

#[derive(Debug, Deserialize)]
struct JsonHandshakePayload {
    device_pubkey: String,
    client_ephemeral_pubkey: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
struct TunnelEnvelope {
    #[serde(rename = "requestId")]
    request_id: String,
    kind: String,
    seq: u64,
    payload: Value,
}

#[derive(Debug, Deserialize)]
struct TunnelRpcPayload {
    method: String,
    #[serde(default)]
    params: Value,
}

async fn handle_peer_online(channel_id: &str) {
    log::info!("[devices/bus] peer online channel_id={}", channel_id);
    PEER_STATUS
        .lock()
        .unwrap()
        .insert(channel_id.to_string(), true);
    // No re-publish: the event was already published by socket::event_handlers.
}

fn handle_peer_offline(channel_id: &str) {
    log::info!("[devices/bus] peer offline channel_id={}", channel_id);
    PEER_STATUS
        .lock()
        .unwrap()
        .insert(channel_id.to_string(), false);
    // No re-publish: the event was already published by socket::event_handlers.
}

/// Handle an incoming `tunnel:frame` — first frame from the device contains its
/// X25519 public key sealed to the core's public key. After successful decryption
/// we derive the shared secret and persist the `PairedDevice`.
async fn handle_tunnel_frame(channel_id: &str, payload_b64: &str) {
    log::debug!(
        "[devices/bus] tunnel:frame channel_id={} payload_len={}",
        channel_id,
        payload_b64.len()
    );

    // Decode the outer base64url envelope.
    let frame_bytes = match crate::openhuman::devices::crypto::base64url_decode(payload_b64) {
        Ok(b) => b,
        Err(e) => {
            log::warn!(
                "[devices/bus] bad base64url in tunnel:frame channel_id={}: {e}",
                channel_id
            );
            return;
        }
    };

    if frame_bytes.first() == Some(&crate::openhuman::devices::crypto::FRAME_VERSION) {
        handle_encrypted_rpc_frame(channel_id, &frame_bytes).await;
        return;
    }

    // Look up the pending keypair for this channel.
    let keypair = {
        let map = PENDING_KEYPAIRS.lock().unwrap();
        map.get(channel_id).cloned()
    };

    let Some(keypair) = keypair else {
        log::debug!(
            "[devices/bus] no pending keypair for channel_id={} — frame ignored",
            channel_id
        );
        return;
    };

    // Wire format for the handshake frame:
    //
    //   0x01 || eph_pub(32) || nonce(24) || ciphertext+tag
    //
    // Version byte 0x01 = "sealed-handshake". The device generates an ephemeral
    // X25519 keypair, performs DH with corePubkey, then seals its static pubkey
    // (32 bytes) with XChaCha20-Poly1305. The core decrypts using the same
    // ephemeral DH to recover the device's static public key, then performs a
    // second DH (core_static ⟷ device_static) for the session key.
    //
    // Version byte 0x02 = "encrypted-frame" (used post-handshake, handled later).
    //
    // Fallback: if the frame begins with a printable ASCII character other than
    // 0x01/0x02, treat the entire payload as a base64url(device_pubkey) string
    // for backward compat with any pre-Layer-2 devices.
    let handshake_payload = if frame_bytes.first() == Some(&0x01) {
        // Sealed handshake: eph_pub(32) || nonce(24) || ciphertext+tag
        if frame_bytes.len() < 1 + 32 + 24 + 16 {
            log::warn!(
                "[devices/bus] sealed-handshake frame too short ({} bytes) channel_id={}",
                frame_bytes.len(),
                channel_id
            );
            return;
        }
        let eph_pub_bytes: [u8; 32] = match frame_bytes[1..33].try_into() {
            Ok(b) => b,
            Err(_) => {
                log::warn!(
                    "[devices/bus] eph_pub slice error channel_id={}",
                    channel_id
                );
                return;
            }
        };
        let core_priv = {
            let map = PENDING_KEYPAIRS.lock().unwrap();
            map.get(channel_id).cloned()
        };
        let Some(core_keypair) = core_priv else {
            log::warn!(
                "[devices/bus] no keypair to open sealed frame channel_id={}",
                channel_id
            );
            return;
        };
        // DH: core_static_priv ⟷ eph_pub → session decryption key.
        let dh_key = match core_keypair.derive_shared_secret(
            &crate::openhuman::devices::crypto::base64url_encode(&eph_pub_bytes),
        ) {
            Ok(k) => k,
            Err(e) => {
                log::warn!(
                    "[devices/bus] DH with eph_pub failed channel_id={}: {e}",
                    channel_id
                );
                return;
            }
        };
        // Decrypt: nonce(24) || ciphertext+tag at offset 33.
        let inner_frame = &frame_bytes[33..];
        match {
            // TunnelCipher::open expects version(1)||nonce(24)||ct+tag, but we already
            // stripped the eph_pub prefix. Reconstruct a plain open call by using
            // XChaCha20 directly on nonce||ct (inner_frame).
            use chacha20poly1305::{
                aead::{Aead, KeyInit},
                XChaCha20Poly1305, XNonce,
            };
            if inner_frame.len() < 24 {
                Err("[devices/bus] inner_frame too short for nonce".to_string())
            } else {
                let nonce = XNonce::from_slice(&inner_frame[..24]);
                let aead = XChaCha20Poly1305::new((&dh_key).into());
                aead.decrypt(nonce, &inner_frame[24..])
                    .map_err(|_| "[devices/bus] AEAD decrypt failed on handshake frame".to_string())
            }
        } {
            Ok(plaintext_bytes) => match String::from_utf8(plaintext_bytes) {
                Ok(s) => parse_handshake_payload(&s),
                Err(_) => {
                    log::warn!(
                        "[devices/bus] decrypted handshake payload is not UTF-8 channel_id={}",
                        channel_id
                    );
                    return;
                }
            },
            Err(e) => {
                log::warn!(
                    "[devices/bus] sealed-handshake decrypt failed channel_id={}: {e}",
                    channel_id
                );
                return;
            }
        }
    } else {
        // Fallback: plaintext base64url-encoded device pubkey (pre-Layer-2 compat).
        log::debug!(
            "[devices/bus] fallback plaintext handshake channel_id={}",
            channel_id
        );
        match String::from_utf8(frame_bytes) {
            Ok(s) => parse_handshake_payload(&s),
            Err(_) => {
                log::warn!(
                    "[devices/bus] tunnel:frame payload not valid UTF-8 for channel_id={}",
                    channel_id
                );
                return;
            }
        }
    };
    let device_pubkey_b64 = handshake_payload.device_pubkey;

    log::info!(
        "[devices/bus] handshake frame received channel_id={} device_pubkey_len={}",
        channel_id,
        device_pubkey_b64.len()
    );

    // Derive shared secret — if this fails the device sent a bad pubkey.
    let static_dh = match keypair.derive_shared_secret(&device_pubkey_b64) {
        Ok(secret) => secret,
        Err(e) => {
            log::error!(
                "[devices/bus] X25519 key agreement failed channel_id={}: {e}",
                channel_id
            );
            return;
        }
    };

    if let Some(client_eph_pubkey) = handshake_payload.client_ephemeral_pubkey {
        if let Err(e) = install_v2_cipher_and_ack(channel_id, &static_dh, &client_eph_pubkey).await
        {
            log::error!(
                "[devices/bus] v2 handshake ack failed channel_id={}: {e}",
                channel_id
            );
            return;
        }
    }

    // Persist the paired device.
    let label = PENDING_SESSIONS
        .lock()
        .unwrap()
        .get(channel_id)
        .map(|s| s.channel_id.clone()) // use channel_id as fallback label
        .unwrap_or_else(|| channel_id.to_string());

    let session_token_hash = hash_session_token(
        &PENDING_SESSIONS
            .lock()
            .unwrap()
            .get(channel_id)
            .map(|s| s.core_session_token.clone())
            .unwrap_or_default(),
    );

    // Load config from global env (best-effort; pairing persists even if config
    // loading is slow — the UI will see the device on next list call).
    if let Ok(config) = crate::openhuman::config::rpc::load_config_with_timeout().await {
        match store::insert_device(
            &config,
            channel_id,
            &label,
            &device_pubkey_b64,
            &session_token_hash,
        ) {
            Ok(device) => {
                log::info!(
                    "[devices/bus] device persisted channel_id={} label={}",
                    device.channel_id,
                    device.label
                );
                publish_global(DomainEvent::DevicePaired {
                    channel_id: channel_id.to_string(),
                    device_pubkey: device_pubkey_b64,
                    label: Some(label),
                });
            }
            Err(e) => {
                log::error!(
                    "[devices/bus] failed to persist device channel_id={}: {e}",
                    channel_id
                );
            }
        }
    } else {
        log::warn!(
            "[devices/bus] could not load config to persist device channel_id={}",
            channel_id
        );
    }
}

fn parse_handshake_payload(raw: &str) -> HandshakePayload {
    let trimmed = raw.trim();
    match serde_json::from_str::<JsonHandshakePayload>(trimmed) {
        Ok(payload) => HandshakePayload {
            device_pubkey: payload.device_pubkey,
            client_ephemeral_pubkey: payload.client_ephemeral_pubkey,
        },
        Err(_) => HandshakePayload {
            device_pubkey: trimmed.to_string(),
            client_ephemeral_pubkey: None,
        },
    }
}

async fn install_v2_cipher_and_ack(
    channel_id: &str,
    static_dh: &[u8; 32],
    client_eph_pubkey_b64: &str,
) -> Result<(), String> {
    let client_eph_bytes = base64url_decode(client_eph_pubkey_b64)
        .map_err(|e| format!("[devices/bus] bad client ephemeral pubkey: {e}"))?;
    if client_eph_bytes.len() != 32 {
        return Err(format!(
            "[devices/bus] client ephemeral pubkey must be 32 bytes, got {}",
            client_eph_bytes.len()
        ));
    }
    let client_eph_arr: [u8; 32] = client_eph_bytes
        .try_into()
        .map_err(|_| "[devices/bus] client ephemeral pubkey slice error".to_string())?;
    let client_eph_pub = PublicKey::from(client_eph_arr);

    let server_eph_secret = StaticSecret::from(rand::random::<[u8; 32]>());
    let server_eph_pub = PublicKey::from(&server_eph_secret);
    let eph_dh = server_eph_secret.diffie_hellman(&client_eph_pub);
    let server_eph_pub_bytes = *server_eph_pub.as_bytes();
    let keys = derive_session_keys(
        static_dh,
        eph_dh.as_bytes(),
        &client_eph_arr,
        &server_eph_pub_bytes,
    );

    ACTIVE_CIPHERS.lock().unwrap().insert(
        channel_id.to_string(),
        Arc::new(std::sync::Mutex::new(TunnelCipher::for_role(
            TunnelRole::Server,
            &keys,
        ))),
    );

    let ack = json!({
        "kind": "handshake_ack",
        "server_ephemeral_pubkey": base64url_encode(&server_eph_pub_bytes),
    });
    let ack_bytes = serde_json::to_vec(&ack)
        .map_err(|e| format!("[devices/bus] handshake ack serialize failed: {e}"))?;
    let bootstrap_cipher = TunnelCipher::new(static_dh);
    let ack_frame = bootstrap_cipher
        .seal(&ack_bytes)
        .map_err(|e| format!("[devices/bus] handshake ack seal failed: {e}"))?;
    let ack_b64 = base64url_encode(&ack_frame);
    emit_frame(channel_id, &ack_b64).await?;
    log::info!(
        "[devices/bus] v2 tunnel cipher installed channel_id={} server_eph_len={}",
        channel_id,
        server_eph_pub_bytes.len()
    );
    Ok(())
}

async fn handle_encrypted_rpc_frame(channel_id: &str, frame_bytes: &[u8]) {
    let cipher = {
        let map = ACTIVE_CIPHERS.lock().unwrap();
        map.get(channel_id).cloned()
    };
    let Some(cipher) = cipher else {
        log::warn!(
            "[devices/bus] encrypted frame with no active cipher channel_id={}",
            channel_id
        );
        return;
    };

    let plaintext = {
        let mut guard = cipher.lock().unwrap();
        match guard.open(frame_bytes) {
            Ok(bytes) => bytes,
            Err(e) => {
                log::warn!(
                    "[devices/bus] encrypted frame open failed channel_id={}: {e}",
                    channel_id
                );
                return;
            }
        }
    };

    let envelope = match serde_json::from_slice::<TunnelEnvelope>(&plaintext) {
        Ok(envelope) => envelope,
        Err(e) => {
            log::warn!(
                "[devices/bus] tunnel envelope parse failed channel_id={}: {e}",
                channel_id
            );
            return;
        }
    };

    if envelope.kind != "request" {
        log::debug!(
            "[devices/bus] ignoring non-request tunnel envelope kind={} channel_id={}",
            envelope.kind,
            channel_id
        );
        return;
    }

    let request = match serde_json::from_value::<TunnelRpcPayload>(envelope.payload) {
        Ok(request) => request,
        Err(e) => {
            emit_tunnel_error(
                channel_id,
                &cipher,
                &envelope.request_id,
                format!("invalid tunnel RPC payload: {e}"),
            )
            .await;
            return;
        }
    };

    log::debug!(
        "[devices/bus] tunnel RPC request channel_id={} method={} request_id={}",
        channel_id,
        request.method,
        envelope.request_id
    );

    let result = crate::core::jsonrpc::invoke_method(
        crate::core::jsonrpc::default_state(),
        &request.method,
        request.params,
    )
    .await;

    match result {
        Ok(value) => {
            emit_tunnel_response(channel_id, &cipher, &envelope.request_id, "response", value)
                .await;
        }
        Err(message) => {
            emit_tunnel_response(
                channel_id,
                &cipher,
                &envelope.request_id,
                "error",
                Value::String(message),
            )
            .await;
        }
    }
}

async fn emit_tunnel_error(
    channel_id: &str,
    cipher: &Arc<std::sync::Mutex<TunnelCipher>>,
    request_id: &str,
    message: String,
) {
    emit_tunnel_response(
        channel_id,
        cipher,
        request_id,
        "error",
        Value::String(message),
    )
    .await;
}

async fn emit_tunnel_response(
    channel_id: &str,
    cipher: &Arc<std::sync::Mutex<TunnelCipher>>,
    request_id: &str,
    kind: &str,
    payload: Value,
) {
    let response = TunnelEnvelope {
        request_id: request_id.to_string(),
        kind: kind.to_string(),
        seq: 0,
        payload,
    };
    let plaintext = match serde_json::to_vec(&response) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::error!(
                "[devices/bus] tunnel response serialize failed channel_id={}: {e}",
                channel_id
            );
            return;
        }
    };
    let encrypted = {
        let guard = cipher.lock().unwrap();
        match guard.seal(&plaintext) {
            Ok(frame) => frame,
            Err(e) => {
                log::error!(
                    "[devices/bus] tunnel response seal failed channel_id={}: {e}",
                    channel_id
                );
                return;
            }
        }
    };
    let payload_b64 = base64url_encode(&encrypted);
    if let Err(e) = emit_frame(channel_id, &payload_b64).await {
        log::error!(
            "[devices/bus] tunnel response emit failed channel_id={}: {e}",
            channel_id
        );
    }
}

/// Resolve the pending `tunnel:register` ack in `tunnel_client`.
fn handle_registered(channel_id: &str, pairing_token: &str, session_token: &str) {
    log::debug!(
        "[devices/bus] tunnel:registered channel_id={} token_len={}",
        channel_id,
        pairing_token.len()
    );
    resolve_register_ack(TunnelRegisterResponse {
        channel_id: channel_id.to_string(),
        pairing_token: pairing_token.to_string(),
        session_token: session_token.to_string(),
    });
}

fn hash_session_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}
