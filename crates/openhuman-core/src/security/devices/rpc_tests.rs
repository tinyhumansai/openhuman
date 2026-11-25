use super::*;
use crate::config::Config;

fn test_config() -> Config {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = Config::default();
    config.workspace_dir = dir.keep();
    config
}

#[test]
fn pairing_begin_requires_tinysecurity_identity_and_receipt() {
    let request = pairing_begin_request("host-event".into());
    assert!(!request.call_id.is_empty());
    assert_eq!(
        request.operation,
        tinysecurity_bus::DevicePairingOperation2::Begin {
            host_event_id: "host-event".into()
        }
    );

    let response = tinysecurity_bus::DevicePairingResponse2 {
        receipt: Some("owner-receipt".into()),
        pairing_id: None,
        session_id: None,
        core_public_key: Some("module-public-key".into()),
        device_public_key: None,
        frame: Vec::new(),
        expires_at_ms: 12_345,
    };

    assert_eq!(
        pairing_begin_result(response).unwrap(),
        ("module-public-key".to_owned(), "owner-receipt".to_owned())
    );

    let incomplete = tinysecurity_bus::DevicePairingResponse2 {
        receipt: None,
        pairing_id: None,
        session_id: None,
        core_public_key: Some("module-public-key".into()),
        device_public_key: None,
        frame: Vec::new(),
        expires_at_ms: 12_345,
    };
    assert!(pairing_begin_result(incomplete).is_err());
}

#[test]
fn pairing_authority_requires_a_trusted_direct_controller_source() {
    let direct =
        crate::core::runtime::DispatchCtx::process_default().with_authenticated_user_controller();
    assert!(require_user_controller(&direct).is_ok());

    let untrusted = crate::core::runtime::DispatchCtx::process_default();
    assert!(require_user_controller(&untrusted).is_err());

    let chat_turn = untrusted.with_turn_origin(
        crate::core::runtime::turn_origin::AgentTurnOrigin::WebChat {
            thread_id: "thread".into(),
            client_id: "client".into(),
            request_id: None,
        },
    );
    assert!(require_user_controller(&chat_turn).is_err());

    let socket_event = untrusted.with_socket_event();
    assert!(require_user_controller(&socket_event).is_err());
}

#[tokio::test]
async fn failed_tunnel_connect_discards_pending_pairing_state() {
    let channel_id = "failed-connect-cleanup";
    PENDING_SESSIONS.lock().unwrap().insert(
        channel_id.to_string(),
        PairingSession {
            channel_id: channel_id.to_string(),
            pairing_token: "token".to_string(),
            connection_nonce: "nonce".to_string(),
            core_pubkey: "public".to_string(),
            pairing_receipt: "owner-receipt".to_string(),
            label: "phone".to_string(),
            rpc_url: None,
            expires_at: "2030-01-01T00:00:00Z".to_string(),
            agent: None,
        },
    );

    let result =
        connect_after_reserving_pairing(channel_id, async { Err("connect failed".to_string()) })
            .await;

    assert_eq!(result, Err("connect failed".to_string()));
    assert!(!PENDING_SESSIONS.lock().unwrap().contains_key(channel_id));
}

#[tokio::test]
async fn devices_list_returns_empty_initially() {
    let config = test_config();
    let result = devices_list(&config).await.unwrap();
    assert!(result.value.devices.is_empty());
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn devices_revoke_nonexistent_returns_false() {
    let config = test_config();
    let ctx =
        crate::core::runtime::DispatchCtx::process_default().with_authenticated_user_controller();
    let result = devices_revoke(&ctx, &config, "NONEXISTENT".to_string())
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
