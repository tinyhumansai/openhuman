use super::*;
use crate::security::devices::types::PairingSession;

fn session(expires_at: &str) -> PairingSession {
    PairingSession {
        channel_id: "bus-test-channel".into(),
        pairing_token: "pairing-token".into(),
        connection_nonce: "bus-test-nonce".into(),
        core_pubkey: "module-public-key".into(),
        pairing_receipt: "owner-receipt".into(),
        label: "phone".into(),
        rpc_url: None,
        expires_at: expires_at.into(),
        agent: None,
    }
}

fn paired_device() -> super::super::types::PairedDevice {
    super::super::types::PairedDevice {
        channel_id: "bus-test-channel".into(),
        label: "phone".into(),
        device_pubkey: "device-static-key".into(),
        created_at: "2026-01-01T00:00:00Z".into(),
        last_seen_at: None,
        peer_online: None,
        revoked: false,
        pairing_id: Some("module-pairing-id".into()),
        core_private_key_encrypted: None,
    }
}

#[test]
fn socket_frame_encoding_round_trips_without_core_crypto() {
    let frame = [0x02, 1, 2, 3, 0xff];
    let encoded = encode_frame(&frame);
    assert_eq!(decode_frame(&encoded).unwrap(), frame);
    assert!(decode_frame("not-base64url!").is_none());
}

#[test]
fn pairing_session_expiry_is_checked_before_handshake_dispatch() {
    assert!(!pairing_session_is_live(&session("2020-01-01T00:00:00Z")));
    assert!(pairing_session_is_live(&session("2099-01-01T00:00:00Z")));
    assert!(!pairing_session_is_live(&session("not-an-expiry")));
}

#[test]
fn first_socket_frame_must_match_begin_channel_token_and_nonce() {
    let session = session("2099-01-01T00:00:00Z");
    assert!(pending_session_matches_socket(
        &session,
        "bus-test-channel",
        "pairing-token",
        "bus-test-nonce"
    ));
    assert!(!pending_session_matches_socket(
        &session,
        "bus-test-channel",
        "pairing-token",
        "attacker-nonce"
    ));
    assert!(!pending_session_matches_socket(
        &session,
        "bus-test-channel",
        "other-token",
        "bus-test-nonce"
    ));
}

#[test]
fn stale_pending_session_is_retired_before_same_device_renewal() {
    let mut pending = std::collections::HashMap::from([(
        "bus-test-channel".to_owned(),
        session("2099-01-01T00:00:00Z"),
    )]);

    // A fresh socket attestation cannot reuse the old pending receipt. Retire
    // that host capability so the handshake path can request TinySecurity
    // Renew for the persisted device identity.
    assert!(take_pending_session_for_socket(
        &mut pending,
        "bus-test-channel",
        "fresh-connection-token",
        "fresh-connection-nonce",
    )
    .0
    .is_none());
    assert!(!pending.contains_key("bus-test-channel"));
    assert!(renewal_pairing_id(true, true, false, false, &paired_device()).is_some());
}

#[test]
fn only_global_credential_clear_is_a_pairing_logout_event() {
    assert!(should_lock_pairings_after_credential_event(
        &DomainEvent::CredentialChanged {
            kind: "cleared".into(),
            tenant: None,
        }
    ));
    assert!(!should_lock_pairings_after_credential_event(
        &DomainEvent::CredentialChanged {
            kind: "session".into(),
            tenant: None,
        }
    ));
    assert!(!should_lock_pairings_after_credential_event(
        &DomainEvent::CredentialChanged {
            kind: "cleared".into(),
            tenant: Some(crate::core::runtime::Tenant::default()),
        }
    ));
}

#[test]
fn tunnel_subscriber_receives_authentication_lifecycle_events() {
    assert_eq!(
        DeviceTunnelSubscriber::new().domains(),
        Some(&["device", "auth"][..])
    );
}

#[test]
fn logout_clears_volatile_tunnel_sessions_and_pairing_handles() {
    let channel = "logout-test-channel".to_owned();
    let scope = "logout-test-scope-012345678901234";
    crate::modules::security::remember_unlocked_pairing_scope(scope, 7);
    PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel.clone(), session("2099-01-01T00:00:00Z"));
    ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(
            channel.clone(),
            super::super::rpc::ActiveDeviceSession {
                scope_id: "scope".into(),
                session_id: "session".into(),
                pairing_token: "token".into(),
                connection_nonce: "nonce".into(),
            },
        );
    PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel.clone(), "pairing-id".into());
    PEER_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel.clone(), true);
    PENDING_HANDSHAKES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .insert(channel.clone());

    clear_device_pairing_runtime_state();

    assert!(!PENDING_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(&channel));
    assert!(!ACTIVE_SESSIONS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(&channel));
    assert!(!PENDING_RENEWALS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(&channel));
    assert!(!PEER_STATUS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains_key(&channel));
    assert!(!PENDING_HANDSHAKES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&channel));
    assert!(!crate::modules::security::pairing_scope_is_unlocked(
        scope, 7
    ));
    assert!(renewal_pairing_id(true, false, false, false, &paired_device()).is_none());
}

#[test]
fn process_restart_unlock_then_peer_online_renews_only_the_same_device() {
    let device = paired_device();
    // Process restart cleared both volatile capabilities. A live persisted
    // row alone cannot authorize renewal until a direct user Unlock succeeds.
    assert!(renewal_pairing_id(false, false, false, false, &device).is_none());
    assert!(renewal_pairing_id(true, false, false, false, &device).is_none());

    // After the direct user path unlocks TinySecurity and the authenticated
    // socket reports this channel online, Renew targets its persisted owner id.
    assert_eq!(
        renewal_pairing_id(true, true, false, false, &device),
        Some("module-pairing-id")
    );
    let attestation_expiry = unix_time_ms().saturating_add(30_000);
    let renewed = renewal_pairing_session(
        &device.channel_id,
        "fresh-socket-token",
        "fresh-socket-nonce",
        &device.label,
        tinysecurity_bus::DevicePairingResponse2 {
            receipt: Some("renewed-receipt".into()),
            pairing_id: None,
            session_id: None,
            core_public_key: Some("module-core-public-key".into()),
            device_public_key: None,
            frame: Vec::new(),
            expires_at_ms: attestation_expiry.saturating_sub(1_000),
        },
        attestation_expiry,
    )
    .expect("TinySecurity Renew should create a bounded pending receipt");
    assert_eq!(renewed.channel_id, device.channel_id);
    assert_eq!(renewed.pairing_token, "fresh-socket-token");
    assert_eq!(renewed.connection_nonce, "fresh-socket-nonce");
    assert_eq!(renewed.pairing_receipt, "renewed-receipt");
    assert!(pairing_session_is_live(&renewed));

    let handshake_reply = valid_handshake_reply(tinysecurity_bus::DevicePairingResponse2 {
        receipt: None,
        pairing_id: None,
        session_id: None,
        core_public_key: None,
        device_public_key: None,
        frame: vec![0x04, 1, 2, 3],
        expires_at_ms: 0,
    })
    .expect("the same-key TinySecurity handshake should return its wire ACK");
    assert_eq!(handshake_reply, [0x04, 1, 2, 3]);
    assert!(renewal_device_matches(
        &device,
        "module-pairing-id",
        "module-pairing-id",
        "device-static-key"
    ));
    assert!(!renewal_device_matches(
        &device,
        "module-pairing-id",
        "other-pairing-id",
        "device-static-key"
    ));
    assert!(!renewal_device_matches(
        &device,
        "module-pairing-id",
        "module-pairing-id",
        "different-device-key"
    ));
    let stale_active = super::super::rpc::ActiveDeviceSession {
        scope_id: "scope".into(),
        session_id: "old-session".into(),
        pairing_token: "old-connection-token".into(),
        connection_nonce: "old-connection-nonce".into(),
    };
    assert!(!active_session_matches_connection(
        &stale_active,
        "fresh-connection-nonce"
    ));
}

#[tokio::test]
async fn uncorrelated_offline_status_does_not_end_the_current_module_session() {
    let channel_id = "offline-status-without-connection-id";
    let active = super::super::rpc::ActiveDeviceSession {
        scope_id: "scope".into(),
        session_id: "module-session".into(),
        pairing_token: "connection-token".into(),
        connection_nonce: "current-connection-nonce".into(),
    };
    ACTIVE_SESSIONS
        .lock()
        .unwrap()
        .insert(channel_id.into(), active.clone());

    handle_peer_offline(channel_id).await;

    assert_eq!(
        ACTIVE_SESSIONS.lock().unwrap().get(channel_id),
        Some(&active)
    );
    assert_eq!(PEER_STATUS.lock().unwrap().get(channel_id), Some(&false));
    ACTIVE_SESSIONS.lock().unwrap().remove(channel_id);
    PEER_STATUS.lock().unwrap().remove(channel_id);
}

#[test]
fn abandoning_pairing_clears_only_host_session_capabilities() {
    let channel_id = "bus-test-channel";
    PENDING_SESSIONS
        .lock()
        .unwrap()
        .insert(channel_id.into(), session("2099-01-01T00:00:00Z"));
    PENDING_HANDSHAKES.lock().unwrap().insert(channel_id.into());
    ACTIVE_SESSIONS.lock().unwrap().insert(
        channel_id.into(),
        super::super::rpc::ActiveDeviceSession {
            scope_id: "scope".into(),
            session_id: "module-session".into(),
            pairing_token: "token".into(),
            connection_nonce: "nonce".into(),
        },
    );

    abandon_unpersisted_pairing(channel_id);

    assert!(!PENDING_SESSIONS.lock().unwrap().contains_key(channel_id));
    assert!(!PENDING_HANDSHAKES.lock().unwrap().contains(channel_id));
    assert!(!ACTIVE_SESSIONS.lock().unwrap().contains_key(channel_id));
}
