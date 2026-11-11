use super::*;

fn pairing_context(
    channel_id: &str,
    source: tinysecurity_bus::DevicePairingSource2,
) -> tinysecurity_bus::DevicePairingContextResponse2 {
    tinysecurity_bus::DevicePairingContextResponse2 {
        scope_id: "0123456789abcdef0123456789abcdef".into(),
        user_binding: "opaque-user-binding".into(),
        secret_master_handle: Some("master-handle".into()),
        legacy_import_handle: None,
        channel_id: channel_id.into(),
        pairing_token: (!channel_id.is_empty()).then(|| "pairing-token".into()),
        connection_nonce: (!channel_id.is_empty())
            .then(|| "connection-nonce".into())
            .unwrap_or_default(),
        generation: 7,
        source,
        expires_at_ms: u64::MAX,
    }
}

#[test]
fn device_pairing_host_event_is_bound_to_purpose_channel_and_used_once() {
    use tinysecurity_bus::{DevicePairingPurpose2, DevicePairingSource2};

    let events = PairingEventRegistry::default();
    let event_id = events
        .issue(
            pairing_context("channel-1", DevicePairingSource2::RegisteredSocket),
            1_000,
        )
        .unwrap();
    let request = tinysecurity_bus::DevicePairingContextRequest2 {
        host_event_id: event_id.clone(),
        purpose: DevicePairingPurpose2::SocketEvent,
        requested_channel_id: Some("channel-1".into()),
    };

    assert_eq!(
        events.resolve(&request, 1_000).unwrap().channel_id,
        "channel-1"
    );
    assert!(
        events.resolve(&request, 1_000).is_err(),
        "event IDs are single use"
    );

    let wrong_purpose = events
        .issue(
            pairing_context("channel-2", DevicePairingSource2::RegisteredSocket),
            1_000,
        )
        .unwrap();
    assert!(events
        .resolve(
            &tinysecurity_bus::DevicePairingContextRequest2 {
                host_event_id: wrong_purpose,
                purpose: DevicePairingPurpose2::Reconnect,
                requested_channel_id: Some("channel-2".into()),
            },
            1_000,
        )
        .is_err());

    let wrong_channel = events
        .issue(
            pairing_context("channel-3", DevicePairingSource2::RegisteredSocket),
            1_000,
        )
        .unwrap();
    assert!(events
        .resolve(
            &tinysecurity_bus::DevicePairingContextRequest2 {
                host_event_id: wrong_channel,
                purpose: DevicePairingPurpose2::SocketEvent,
                requested_channel_id: Some("attacker-channel".into()),
            },
            1_000,
        )
        .is_err());
}

#[test]
fn direct_recovery_event_is_bound_to_its_exact_channel() {
    use tinysecurity_bus::{DevicePairingPurpose2, DevicePairingSource2};

    let events = PairingEventRegistry::default();
    let recovery_context = || {
        let mut context = pairing_context(
            "recover-channel",
            DevicePairingSource2::AuthenticatedUserController,
        );
        context.pairing_token = None;
        context.connection_nonce.clear();
        context
    };
    let event = events.issue(recovery_context(), 1_000).unwrap();
    let request = |channel: &str| tinysecurity_bus::DevicePairingContextRequest2 {
        host_event_id: event.clone(),
        purpose: DevicePairingPurpose2::UserController,
        requested_channel_id: Some(channel.to_owned()),
    };

    assert_eq!(
        events
            .resolve(&request("recover-channel"), 1_000)
            .unwrap()
            .channel_id,
        "recover-channel"
    );

    let wrong_channel = events.issue(recovery_context(), 1_000).unwrap();
    assert!(events
        .resolve(
            &tinysecurity_bus::DevicePairingContextRequest2 {
                host_event_id: wrong_channel,
                purpose: DevicePairingPurpose2::UserController,
                requested_channel_id: Some("other-channel".into()),
            },
            1_000,
        )
        .is_err());
}

#[test]
fn authenticated_logout_event_has_no_socket_or_pairing_fields() {
    use tinysecurity_bus::{DevicePairingPurpose2, DevicePairingSource2};

    let events = PairingEventRegistry::default();
    let mut context = pairing_context("", DevicePairingSource2::AuthenticatedUserLogout);
    context.channel_id.clear();
    context.pairing_token = None;
    context.connection_nonce.clear();
    let event = events.issue(context, 1_000).unwrap();
    let request = tinysecurity_bus::DevicePairingContextRequest2 {
        host_event_id: event,
        purpose: DevicePairingPurpose2::UserLogout,
        requested_channel_id: None,
    };
    assert!(events.resolve(&request, 1_000).is_ok());

    let mut forged = pairing_context(
        "logout-channel",
        DevicePairingSource2::AuthenticatedUserLogout,
    );
    forged.pairing_token = None;
    forged.connection_nonce = "socket-nonce".into();
    assert!(events.issue(forged, 1_000).is_err());
}

#[tokio::test]
async fn device_pairing_host_is_served_on_the_module_binding() {
    use tinybus::broker::Broker;
    use tinybus::module::{HostCallbackEndpoint, ModuleHost};
    use tinybus::transport::memory::MemoryBus;
    use tinysecurity_bus::callback_names2;

    let broker = Broker::new();
    let bus = MemoryBus::new();
    let _broker_task = broker.spawn(bus);
    let host = ModuleHost::new(broker);
    let path: tinybus::ObjectPath = callback_names2::OBJECT_PATH.parse().unwrap();
    let endpoint = |interface: &str, members: &[&str]| HostCallbackEndpoint {
        path: path.clone(),
        interface: interface.parse().unwrap(),
        members: members
            .iter()
            .map(|member| member.parse().unwrap())
            .collect(),
    };
    let binding = host
        .prepare_host_callbacks(vec![
            endpoint(
                callback_names2::APPROVAL_STORE,
                &[callback_names2::LOAD, callback_names2::CAS],
            ),
            endpoint(callback_names2::AUDIT_SINK, &[callback_names2::APPEND]),
            endpoint(
                callback_names2::APPROVAL_PROMPT,
                &[callback_names2::PROMPT, callback_names2::VERIFY_HUMAN],
            ),
            endpoint(
                callback_names2::KEYRING_CONSENT,
                &[callback_names2::CONSENT],
            ),
            endpoint(
                callback_names2::DEVICE_PAIRING_HOST,
                &[
                    callback_names2::RESOLVE_PAIRING_EVENT,
                    callback_names2::PERSIST_DEVICE_PAIRING,
                    callback_names2::LOAD_LEGACY_DEVICE_IDENTITIES,
                ],
            ),
        ])
        .await
        .unwrap();
    serve_on_binding(&binding).await.unwrap();

    assert!(
        binding.serve_at(path, DevicePairingHost).await.is_err(),
        "DevicePairingHost must already be registered on this binding"
    );
}

#[test]
fn direct_controller_pairing_event_requires_the_registered_channel_token_and_nonce() {
    use tinysecurity_bus::{DevicePairingPurpose2, DevicePairingSource2};

    let events = PairingEventRegistry::default();
    let context = pairing_context(
        "channel-begin",
        DevicePairingSource2::AuthenticatedUserController,
    );
    let event = events.issue(context, 1_000).unwrap();
    assert!(events
        .resolve(
            &tinysecurity_bus::DevicePairingContextRequest2 {
                host_event_id: event,
                purpose: DevicePairingPurpose2::UserController,
                requested_channel_id: None,
            },
            1_000,
        )
        .is_ok());

    let mut incomplete = pairing_context(
        "channel-incomplete",
        DevicePairingSource2::AuthenticatedUserController,
    );
    incomplete.connection_nonce.clear();
    assert!(events.issue(incomplete, 1_000).is_err());
}

#[test]
fn keyring_callback_grants_only_explicit_operation_scoped_approval() {
    use tinysecurity_bus::KeyringConsentOutcome2 as Outcome;
    assert_eq!(keyring_consent_outcome(Some(true)), Outcome::Allowed);
    assert_eq!(keyring_consent_outcome(Some(false)), Outcome::ConsentDenied);
    assert_eq!(keyring_consent_outcome(None), Outcome::Unavailable);
}

#[test]
fn unmigrated_callbacks_return_an_error_instead_of_authority() {
    assert!(unavailable()
        .to_string()
        .contains("security host callback unavailable"));
}

#[test]
fn legacy_import_handle_is_bound_to_source_and_consumed_once() {
    let handle = uuid::Uuid::new_v4().simple().to_string();
    register_legacy_import_grant(
        handle.clone(),
        crate::config::Config::default(),
        "0123456789abcdef0123456789abcdef".into(),
        "opaque-user-binding".into(),
        7,
        "abcdef0123456789abcdef0123456789".into(),
        "fedcba9876543210fedcba9876543210".into(),
        u64::MAX,
    )
    .unwrap();

    let grant = take_legacy_import_grant(&handle, 1).expect("live one-shot grant");
    assert_eq!(grant.scope_id, "0123456789abcdef0123456789abcdef");
    assert_eq!(grant.generation, 7);
    assert_eq!(grant.backend_id, "abcdef0123456789abcdef0123456789");
    assert!(
        take_legacy_import_grant(&handle, 1).is_none(),
        "grant replay denied"
    );

    assert!(register_legacy_import_grant(
        uuid::Uuid::new_v4().simple().to_string(),
        crate::config::Config::default(),
        "0123456789abcdef0123456789abcdef".into(),
        "opaque-user-binding".into(),
        7,
        "not-a-backend".into(),
        "fedcba9876543210fedcba9876543210".into(),
        u64::MAX,
    )
    .is_err());
}

#[tokio::test]
async fn pairing_callbacks_fail_closed_without_an_admitted_host_event() {
    let host = DevicePairingHost;
    let result = host
        .resolve_pairing_event(tinysecurity_bus::DevicePairingContextRequest2 {
            host_event_id: "unissued-event".into(),
            purpose: tinysecurity_bus::DevicePairingPurpose2::UserController,
            requested_channel_id: None,
        })
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("security host callback unavailable"));
}

#[tokio::test]
async fn pairing_persistence_callbacks_fail_closed_without_a_bound_scope() {
    let host = DevicePairingHost;
    let result = host
        .persist_device_pairing(tinysecurity_bus::DevicePairingPersistenceRequest2 {
            backend_id: "unconfigured".into(),
            scope_id: "unconfigured".into(),
            secret_registration_id: "unconfigured".into(),
            operation: tinysecurity_bus::DevicePairingPersistenceOperation2::Load,
        })
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("security host callback unavailable"));
}

#[tokio::test]
async fn legacy_pairing_import_fails_closed_without_a_one_shot_authority() {
    let host = DevicePairingHost;
    let result = host
        .load_legacy_device_identities(tinysecurity_bus::DevicePairingLegacyLoadRequest2 {
            handle: "unissued-import".into(),
        })
        .await;
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("security host callback unavailable"));
}
