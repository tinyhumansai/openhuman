//! Event bus handlers for the devices domain.
//!
//! The host owns the socket transport. TinySecurity owns pairing identities,
//! handshake state, session ciphers and every per-frame crypto operation.

use crate::core::runtime::DispatchCtx;
use std::sync::{Arc, OnceLock};

#[cfg(feature = "security-module")]
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::devices::rpc::{
    ACTIVE_SESSIONS, PEER_STATUS, PENDING_RENEWALS, PENDING_SESSIONS, RECONNECTING,
};
#[cfg(feature = "security-module")]
use crate::security::devices::store;
#[cfg(feature = "security-module")]
use crate::security::devices::tunnel_client::emit_frame;
use async_trait::async_trait;
#[cfg(feature = "security-module")]
use serde_json::Value;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;
#[cfg(feature = "security-module")]
use tunnel_rpc::{decode_frame, encode_frame};

const HANDSHAKE_VERSION: u8 = 0x03;

static DEVICE_TUNNEL_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();
static DEVICE_TUNNEL_SHUTDOWN: OnceLock<()> = OnceLock::new();

/// Register the device tunnel subscriber on the global event bus.
/// Idempotent — subsequent calls are no-ops.
pub fn register_device_tunnel_subscriber() {
    if DEVICE_TUNNEL_HANDLE.get().is_some() {
        return;
    }
    match crate::core::bus::BUS.subscribe(Arc::new(DeviceTunnelSubscriber::new())) {
        Some(handle) => {
            let _ = DEVICE_TUNNEL_HANDLE.set(handle);
            if DEVICE_TUNNEL_SHUTDOWN.set(()).is_ok() {
                crate::core::shutdown::register(|| async {
                    lock_device_pairings_after_logout().await;
                });
            }
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
impl EventHandler<DomainEvent> for DeviceTunnelSubscriber {
    fn name(&self) -> &str {
        "device::tunnel"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["device", "auth"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::DevicePeerOnline { channel_id } => {
                handle_peer_online(channel_id).await;
            }
            DomainEvent::DevicePeerOffline { channel_id } => {
                handle_peer_offline(channel_id).await;
            }
            DomainEvent::DeviceTunnelFrame {
                channel_id,
                payload_b64,
                connection_token,
                connection_nonce,
                expires_at_ms,
            } => {
                let pending = PENDING_SESSIONS
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .get(channel_id.as_str())
                    .cloned();
                let host_ctx = DispatchCtx::host_default();
                match super::owner::owner_of(&host_ctx, channel_id, pending.as_ref()).await {
                    Ok(owner) => {
                        let handled = crate::storage::agents::within_agent(
                            &host_ctx,
                            owner.as_deref(),
                            async {
                                let owner_ctx = DispatchCtx::ambient().with_socket_event();
                                handle_tunnel_frame(
                                    &owner_ctx,
                                    channel_id,
                                    payload_b64,
                                    connection_token,
                                    connection_nonce,
                                    *expires_at_ms,
                                )
                                .await
                            },
                        )
                        .await;
                        if handled.is_none() {
                            log::warn!(
                                "[devices/bus] dropping tunnel frame channel_id={channel_id}: \n                                 its owner has no context to act under"
                            );
                        }
                    }
                    Err(failed) => log::warn!(
                        "[devices/bus] dropping tunnel frame channel_id={channel_id}: owner \n                         lookup failed in scope={} ({})",
                        failed.agent.as_deref().unwrap_or("local"),
                        failed.error
                    ),
                }
            }
            event if should_lock_pairings_after_credential_event(event) => {
                clear_device_pairing_runtime_state();
            }
            _ => {}
        }
    }
}

fn should_lock_pairings_after_credential_event(event: &DomainEvent) -> bool {
    matches!(
        event,
        DomainEvent::CredentialChanged {
            kind,
            tenant: None,
        } if kind == "cleared"
    )
}

async fn lock_device_pairings_after_logout() {
    clear_device_pairing_runtime_state();
    #[cfg(feature = "security-module")]
    {
        let host_context = DispatchCtx::host_default();
        let Ok(config) = host_context.load_config().await else {
            log::warn!("[devices/bus] unable to load config to lock TinySecurity after logout");
            return;
        };
        if let Err(error) = crate::modules::security::lock_device_pairing_scope(&config).await {
            log::warn!("[devices/bus] TinySecurity pairing lock after logout failed: {error}");
        }
    }
}

/// Lock the currently admitted TinySecurity scope before credential teardown
/// deactivates or rebinds it. The direct credential lifecycle owns this call;
/// socket callbacks cannot mint logout authority.
pub(crate) async fn lock_device_pairing_scope_for_logout(
    _ctx: &DispatchCtx,
    config: &crate::config::Config,
) {
    clear_device_pairing_runtime_state();
    #[cfg(feature = "security-module")]
    {
        if let Err(error) = crate::modules::security::lock_device_pairing_scope(config).await {
            log::warn!("[devices/bus] TinySecurity pairing lock before logout failed: {error}");
        }
    }
    #[cfg(not(feature = "security-module"))]
    let _ = (config, _ctx);
}

fn clear_device_pairing_runtime_state() {
    PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    PEER_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    RECONNECTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    PENDING_HANDSHAKES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
    #[cfg(feature = "security-module")]
    crate::modules::security::clear_device_pairing_unlock_cache();
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn handle_peer_online(channel_id: &str) {
    log::info!("[devices/bus] peer online channel_id={}", channel_id);
    super::lock_unpoisoned(&PEER_STATUS).insert(channel_id.to_string(), true);
    // No re-publish: the event was already published by socket::event_handlers.
}

async fn handle_peer_offline(channel_id: &str) {
    log::info!("[devices/bus] peer offline channel_id={}", channel_id);
    super::lock_unpoisoned(&PEER_STATUS).insert(channel_id.to_string(), false);
    // This backend event carries only a channel id, not the closed socket's
    // connection nonce. Keep the module session until expiry rather than
    // attesting a close against whichever connection is current now.
}

/// Route a socket frame through TinySecurity and preserve the existing wire envelope.
#[cfg(feature = "security-module")]
async fn handle_tunnel_frame(
    ctx: &DispatchCtx,
    channel_id: &str,
    payload_b64: &str,
    connection_token: &str,
    connection_nonce: &str,
    expires_at_ms: u64,
) {
    let Some(frame_bytes) = decode_frame(payload_b64) else {
        log::warn!("[devices/bus] invalid tunnel frame encoding channel_id={channel_id}");
        return;
    };
    let Ok(config) = ctx.load_config().await else {
        return;
    };
    if frame_bytes.first() == Some(&HANDSHAKE_VERSION) {
        handle_handshake(
            channel_id,
            &config,
            &frame_bytes,
            connection_token,
            connection_nonce,
            expires_at_ms,
        )
        .await;
    } else if frame_bytes.first() == Some(&0x02) {
        tunnel_rpc::handle_encrypted_rpc_frame(
            ctx,
            channel_id,
            &config,
            &frame_bytes,
            connection_token,
            connection_nonce,
            expires_at_ms,
        )
        .await;
    } else {
        log::warn!("[devices/bus] unsupported frame version channel_id={channel_id}");
    }
}

#[cfg(not(feature = "security-module"))]
async fn handle_tunnel_frame(
    _ctx: &DispatchCtx,
    channel_id: &str,
    _payload_b64: &str,
    _connection_token: &str,
    _connection_nonce: &str,
    _expires_at_ms: u64,
) {
    log::debug!("[devices/bus] tunnel disabled without security-module channel_id={channel_id}");
}

#[cfg(feature = "security-module")]
async fn handle_handshake(
    channel_id: &str,
    config: &crate::config::Config,
    frame: &[u8],
    connection_token: &str,
    connection_nonce: &str,
    expires_at_ms: u64,
) {
    let (pending, stale_pending) = take_pending_session_for_socket(
        &mut PENDING_SESSIONS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
        channel_id,
        connection_token,
        connection_nonce,
    );
    if stale_pending {
        PENDING_RENEWALS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(channel_id);
    }
    let session = match pending {
        Some(session) => session,
        None => {
            let Some(session) = begin_device_renewal(
                channel_id,
                config,
                connection_token,
                connection_nonce,
                expires_at_ms,
            )
            .await
            else {
                log::warn!("[devices/bus] no pending pairing or renew authority for channel_id={channel_id}");
                return;
            };
            session
        }
    };
    if !pending_session_matches_socket(&session, channel_id, connection_token, connection_nonce)
        || !pairing_session_is_live(&session)
    {
        return;
    }
    let Ok((event_id, _scope_id)) = crate::modules::security::device_pairing_socket_event(
        config,
        channel_id,
        &session.pairing_token,
        connection_nonce,
        expires_at_ms,
    )
    .await
    else {
        return;
    };
    let Ok(client) = crate::modules::security::client(config).await else {
        return;
    };
    let result = client
        .device_pairing2(tinysecurity_bus::DevicePairingRequest2 {
            call_id: uuid::Uuid::new_v4().simple().to_string(),
            operation: tinysecurity_bus::DevicePairingOperation2::Handshake {
                receipt: session.pairing_receipt.clone(),
                host_event_id: event_id,
                frame: frame.to_vec(),
            },
        })
        .await;
    let Ok(result) = result else {
        log::warn!("[devices/bus] TinySecurity rejected handshake channel_id={channel_id}");
        return;
    };
    let Some(ack) = valid_handshake_reply(result) else {
        log::warn!("[devices/bus] malformed TinySecurity handshake result channel_id={channel_id}");
        return;
    };
    if let Err(error) = emit_frame(channel_id, &encode_frame(&ack)).await {
        log::warn!("[devices/bus] handshake ACK emit failed channel_id={channel_id}: {error}");
        return;
    }
    PENDING_HANDSHAKES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel_id.to_owned());
}

#[cfg(feature = "security-module")]
async fn begin_device_renewal(
    channel_id: &str,
    config: &crate::config::Config,
    connection_token: &str,
    connection_nonce: &str,
    expires_at_ms: u64,
) -> Option<super::types::PairingSession> {
    if !PEER_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .copied()
        .unwrap_or(false)
    {
        return None;
    }
    if !RECONNECTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel_id.to_owned())
    {
        return None;
    }

    let renewed = begin_device_renewal_once(
        channel_id,
        config,
        connection_token,
        connection_nonce,
        expires_at_ms,
    )
    .await;
    RECONNECTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
    renewed
}

#[cfg(feature = "security-module")]
async fn begin_device_renewal_once(
    channel_id: &str,
    config: &crate::config::Config,
    connection_token: &str,
    connection_nonce: &str,
    expires_at_ms: u64,
) -> Option<super::types::PairingSession> {
    let device = match store::get_device(config, channel_id) {
        Ok(Some(device)) if !device.revoked => device,
        _ => return None,
    };
    let is_pending = PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(channel_id);
    let active_is_current = ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .is_some_and(|active| active_session_matches_connection(active, connection_nonce));
    let peer_online = PEER_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(channel_id)
        .copied()
        .unwrap_or(false);
    let scope_unlocked = crate::modules::security::device_pairing_scope_is_unlocked(config).await;
    let pairing_id = renewal_pairing_id(
        peer_online,
        scope_unlocked,
        is_pending,
        active_is_current,
        &device,
    )?;
    let event_id = crate::modules::security::device_pairing_reconnect_event(
        config,
        channel_id,
        connection_token,
        connection_nonce,
        expires_at_ms,
    )
    .await
    .ok()?;
    let client = crate::modules::security::client(config).await.ok()?;
    let response = client
        .device_pairing2(tinysecurity_bus::DevicePairingRequest2 {
            call_id: uuid::Uuid::new_v4().simple().to_string(),
            operation: tinysecurity_bus::DevicePairingOperation2::Renew {
                pairing_id: pairing_id.to_owned(),
                host_event_id: event_id,
            },
        })
        .await
        .ok()?;
    let session = renewal_pairing_session(
        channel_id,
        connection_token,
        connection_nonce,
        &device.label,
        response,
        expires_at_ms,
    )?;
    PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel_id.to_owned(), pairing_id.to_owned());
    PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel_id.to_owned(), session.clone());
    Some(session)
}

fn renewal_pairing_id(
    peer_online: bool,
    scope_unlocked: bool,
    has_pending_session: bool,
    has_current_session: bool,
    device: &super::types::PairedDevice,
) -> Option<&str> {
    if !peer_online
        || !scope_unlocked
        || has_pending_session
        || has_current_session
        || device.revoked
    {
        return None;
    }
    device
        .pairing_id
        .as_deref()
        .filter(|value| !value.is_empty())
}

fn renewal_pairing_session(
    channel_id: &str,
    connection_token: &str,
    connection_nonce: &str,
    label: &str,
    response: tinysecurity_bus::DevicePairingResponse2,
    attestation_expiry_ms: u64,
) -> Option<super::types::PairingSession> {
    let receipt = response.receipt.filter(|value| !value.is_empty())?;
    let core_pubkey = response.core_public_key.filter(|value| !value.is_empty())?;
    if response.pairing_id.is_some()
        || response.session_id.is_some()
        || response.device_public_key.is_some()
        || !response.frame.is_empty()
        || response.expires_at_ms <= unix_time_ms()
        || response.expires_at_ms > attestation_expiry_ms
    {
        return None;
    }
    let expires_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(
        i64::try_from(response.expires_at_ms).ok()?,
    )?
    .to_rfc3339();
    Some(super::types::PairingSession {
        channel_id: channel_id.to_owned(),
        pairing_token: connection_token.to_owned(),
        connection_nonce: connection_nonce.to_owned(),
        core_pubkey,
        pairing_receipt: receipt,
        label: label.to_owned(),
        rpc_url: None,
        expires_at,
        agent: None,
    })
}

fn renewal_device_matches(
    existing: &super::types::PairedDevice,
    expected_pairing_id: &str,
    returned_pairing_id: &str,
    returned_device_key: &str,
) -> bool {
    !existing.revoked
        && existing.pairing_id.as_deref() == Some(expected_pairing_id)
        && returned_pairing_id == expected_pairing_id
        && existing.device_pubkey == returned_device_key
}

fn active_session_matches_connection(
    active: &super::rpc::ActiveDeviceSession,
    connection_nonce: &str,
) -> bool {
    active.connection_nonce == connection_nonce
}

fn valid_handshake_reply(response: tinysecurity_bus::DevicePairingResponse2) -> Option<Vec<u8>> {
    if response.frame.is_empty()
        || response.receipt.is_some()
        || response.pairing_id.is_some()
        || response.session_id.is_some()
        || response.core_public_key.is_some()
        || response.device_public_key.is_some()
    {
        None
    } else {
        Some(response.frame)
    }
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

static PENDING_HANDSHAKES: once_cell::sync::Lazy<
    std::sync::Mutex<std::collections::HashSet<String>>,
> = once_cell::sync::Lazy::new(Default::default);

fn pairing_session_is_live(session: &super::types::PairingSession) -> bool {
    chrono::DateTime::parse_from_rfc3339(&session.expires_at)
        .ok()
        .is_some_and(|expiry| expiry.timestamp_millis() > chrono::Utc::now().timestamp_millis())
}

fn pending_session_matches_socket(
    session: &super::types::PairingSession,
    channel_id: &str,
    connection_token: &str,
    connection_nonce: &str,
) -> bool {
    session.channel_id == channel_id
        && session.pairing_token == connection_token
        && session.connection_nonce == connection_nonce
}

fn take_pending_session_for_socket(
    pending: &mut std::collections::HashMap<String, super::types::PairingSession>,
    channel_id: &str,
    connection_token: &str,
    connection_nonce: &str,
) -> (Option<super::types::PairingSession>, bool) {
    let Some(session) = pending.get(channel_id).cloned() else {
        return (None, false);
    };
    if pending_session_matches_socket(&session, channel_id, connection_token, connection_nonce)
        && pairing_session_is_live(&session)
    {
        (Some(session), false)
    } else {
        pending.remove(channel_id);
        (None, true)
    }
}

pub(super) fn abandon_unpersisted_pairing(channel_id: &str) {
    PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
    PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
    PENDING_HANDSHAKES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
    ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
    PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
    RECONNECTING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(channel_id);
}

fn hash_session_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(feature = "security-module")]
#[path = "bus/tunnel_rpc.rs"]
mod tunnel_rpc;

#[cfg(all(test, feature = "security-module"))]
#[path = "bus_tests.rs"]
mod tests;
