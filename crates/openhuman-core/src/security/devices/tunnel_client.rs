//! Core-owned semantic port for the hosted device tunnel client.
//!
//! The host owns Socket.IO event names, payloads, ACK parsing, and connection
//! details. Core device operations use this typed interface and consume
//! semantic peer/frame events only.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

#[derive(Clone)]
struct SocketAttestation {
    token: String,
    nonce: String,
    expires_at_ms: u64,
    preserve_first_online: bool,
}

static SOCKET_ATTESTATIONS: once_cell::sync::Lazy<
    std::sync::Mutex<HashMap<String, SocketAttestation>>,
> = once_cell::sync::Lazy::new(Default::default);

use async_trait::async_trait;

/// Registration details needed by the device pairing domain after the host
/// parses the backend's `tunnel:register` ACK.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TunnelRegistration {
    pub channel_id: String,
    pub pairing_token: String,
    /// ISO 8601 timestamp regardless of the backend's wire representation.
    pub pairing_expires_at: String,
}

/// Host implementation of the hosted device tunnel transport.
#[async_trait]
pub trait TunnelClientPort: Send + Sync + 'static {
    async fn register(&self) -> Result<TunnelRegistration, String>;
    async fn connect(&self, channel_id: &str) -> Result<(), String>;
    async fn emit_frame(&self, channel_id: &str, payload_b64: &str) -> Result<(), String>;
}

static TUNNEL_CLIENT_PORT: OnceLock<Arc<dyn TunnelClientPort>> = OnceLock::new();

/// Install the host's tunnel client. Duplicate installation leaves the first
/// process-wide adapter in place, matching the other core host ports.
pub fn set_global_tunnel_client_port(port: Arc<dyn TunnelClientPort>) {
    if TUNNEL_CLIENT_PORT.set(port).is_err() {
        log::warn!("[devices/tunnel] tunnel client port already installed; ignoring duplicate");
    }
}

/// Return the host's registered tunnel client, if one is installed.
pub fn global_tunnel_client_port() -> Option<Arc<dyn TunnelClientPort>> {
    TUNNEL_CLIENT_PORT.get().cloned()
}

fn require_port() -> Result<Arc<dyn TunnelClientPort>, String> {
    global_tunnel_client_port()
        .ok_or_else(|| "[devices/tunnel] SocketManager not initialized".to_string())
}

/// Register a pairing channel through the host-owned backend tunnel.
pub async fn emit_register() -> Result<TunnelRegistration, String> {
    require_port()?.register().await
}

/// Ask the host to begin listening on a channel as the core role.
pub async fn emit_connect(channel_id: &str) -> Result<(), String> {
    require_port()?.connect(channel_id).await
}

/// Send an encrypted device frame through the host-owned backend tunnel.
pub async fn emit_frame(channel_id: &str, payload_b64: &str) -> Result<(), String> {
    require_port()?.emit_frame(channel_id, payload_b64).await
}

/// Publish a parsed peer-presence update from the host adapter.
pub fn publish_peer_status(channel_id: String, online: bool) {
    if online {
        let mut attestations = SOCKET_ATTESTATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match attestations.get_mut(&channel_id) {
            Some(attestation) if attestation.preserve_first_online => {
                attestation.preserve_first_online = false;
                attestation.expires_at_ms = event_expiry_ms();
            }
            Some(attestation) => {
                *attestation = SocketAttestation {
                    token: uuid::Uuid::new_v4().simple().to_string(),
                    nonce: uuid::Uuid::new_v4().simple().to_string(),
                    expires_at_ms: event_expiry_ms(),
                    preserve_first_online: false,
                };
            }
            None => {
                attestations.insert(
                    channel_id.clone(),
                    SocketAttestation {
                        token: uuid::Uuid::new_v4().simple().to_string(),
                        nonce: uuid::Uuid::new_v4().simple().to_string(),
                        expires_at_ms: event_expiry_ms(),
                        preserve_first_online: false,
                    },
                );
            }
        }
    } else {
        // The backend status has no peer-connection id. Retire the host token
        // so a later frame cannot inherit an ambiguous old connection binding.
        SOCKET_ATTESTATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&channel_id);
    }
    let event = if online {
        crate::core::events::DomainEvent::DevicePeerOnline { channel_id }
    } else {
        crate::core::events::DomainEvent::DevicePeerOffline { channel_id }
    };
    crate::core::bus::BUS.publish(event);
}

/// Bind the first socket connection to the fresh Begin receipt before the
/// host asks the backend to connect the registered channel.
pub(super) fn seed_pairing_socket_attestation(
    channel_id: &str,
    pairing_token: &str,
    connection_nonce: &str,
) -> Result<(), String> {
    if channel_id.is_empty()
        || channel_id.len() > 128
        || pairing_token.is_empty()
        || pairing_token.len() > 512
        || connection_nonce.is_empty()
        || connection_nonce.len() > 128
    {
        return Err("invalid pairing socket attestation".to_owned());
    }
    SOCKET_ATTESTATIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            channel_id.to_owned(),
            SocketAttestation {
                token: pairing_token.to_owned(),
                nonce: connection_nonce.to_owned(),
                expires_at_ms: event_expiry_ms(),
                preserve_first_online: true,
            },
        );
    Ok(())
}

/// Publish a parsed encrypted frame from the host adapter.
pub fn publish_tunnel_frame(channel_id: String, payload_b64: String) {
    let attestation = {
        let mut attestations = SOCKET_ATTESTATIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = attestations
            .entry(channel_id.clone())
            .or_insert_with(|| SocketAttestation {
                token: uuid::Uuid::new_v4().simple().to_string(),
                nonce: uuid::Uuid::new_v4().simple().to_string(),
                expires_at_ms: event_expiry_ms(),
                preserve_first_online: false,
            });
        entry.expires_at_ms = event_expiry_ms();
        entry.clone()
    };
    crate::core::bus::BUS.publish(crate::core::events::DomainEvent::DeviceTunnelFrame {
        channel_id,
        payload_b64,
        connection_token: attestation.token,
        connection_nonce: attestation.nonce,
        expires_at_ms: attestation.expires_at_ms,
    });
}

fn event_expiry_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
        .saturating_add(30_000)
}

#[cfg(test)]
#[path = "tunnel_client_tests.rs"]
mod tests;
