use super::*;
use crate::core::runtime::DispatchCtx;

fn session(agent: Option<&str>) -> PairingSession {
    PairingSession {
        channel_id: "owner-test".to_string(),
        pairing_token: "token".to_string(),
        core_pubkey: "pk".to_string(),
        rpc_url: None,
        expires_at: "2099-01-01T00:00:00Z".to_string(),
        agent: agent.map(str::to_string),
    }
}

#[tokio::test]
async fn a_pending_pairing_names_its_agent() {
    let pending = session(Some("agent-7"));
    assert_eq!(
        owner_of(
            &DispatchCtx::process_default(),
            "owner-test-pending",
            Some(&pending)
        )
        .await
        .unwrap()
        .as_deref(),
        Some("agent-7")
    );
    let local = session(None);
    assert_eq!(
        owner_of(
            &DispatchCtx::process_default(),
            "owner-test-pending",
            Some(&local)
        )
        .await,
        Ok(None)
    );
}

/// A remembered owner is re-checked in its scope: once that scope no longer
/// holds the device (revoked, deleted, re-paired elsewhere) the entry is
/// forgotten and the channel resolved afresh.
#[tokio::test]
async fn a_remembered_owner_that_lost_the_device_is_forgotten() {
    let tmp = tempfile::tempdir().unwrap();
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        scoped_config(tmp.path()),
    );
    remember("owner-test-stale", None);
    let owner = crate::core::runtime::CoreContext::scope(
        context.clone(),
        owner_of(&DispatchCtx::new(context.clone()), "owner-test-stale", None),
    )
    .await;
    assert_eq!(owner, Ok(None));
    assert_eq!(cached("owner-test-stale"), None, "the stale entry is gone");
}

#[tokio::test]
async fn a_revoked_device_is_not_its_agents_any_more() {
    let tmp = tempfile::tempdir().unwrap();
    let config = scoped_config(tmp.path());
    super::super::store::insert_device(&config, "owner-test-revoked", "label", "pk", "hash")
        .unwrap();
    super::super::store::revoke_device(&config, "owner-test-revoked").unwrap();
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let owner = crate::core::runtime::CoreContext::scope(
        context.clone(),
        owner_of(
            &DispatchCtx::new(context.clone()),
            "owner-test-revoked",
            None,
        ),
    )
    .await;
    assert_eq!(owner, Ok(None));
    assert_eq!(
        cached("owner-test-revoked"),
        None,
        "a revoked device is not remembered"
    );
}

#[test]
fn the_agent_is_not_sent_over_the_wire_when_absent() {
    let json = serde_json::to_value(session(None)).unwrap();
    assert!(json.get("agent").is_none());
    let json = serde_json::to_value(session(Some("a"))).unwrap();
    assert_eq!(json["agent"], "a");
}

fn scoped_config(dir: &std::path::Path) -> crate::config::Config {
    crate::config::Config {
        workspace_dir: dir.join("workspace"),
        config_path: dir.join("config.toml"),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_device_found_in_the_local_scope_belongs_to_local_and_is_remembered() {
    let tmp = tempfile::tempdir().unwrap();
    let config = scoped_config(tmp.path());
    super::super::store::insert_device(&config, "owner-test-found", "label", "pk", "hash").unwrap();
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config,
    );
    let owner = crate::core::runtime::CoreContext::scope(
        context.clone(),
        owner_of(&DispatchCtx::new(context.clone()), "owner-test-found", None),
    )
    .await;
    assert_eq!(owner, Ok(None));
    assert_eq!(cached("owner-test-found"), Some(None));
}

#[tokio::test]
async fn a_channel_no_scope_knows_is_local_and_not_remembered() {
    let tmp = tempfile::tempdir().unwrap();
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        scoped_config(tmp.path()),
    );
    let owner = crate::core::runtime::CoreContext::scope(
        context.clone(),
        owner_of(
            &DispatchCtx::new(context.clone()),
            "owner-test-missing",
            None,
        ),
    )
    .await;
    assert_eq!(owner, Ok(None));
    assert_eq!(cached("owner-test-missing"), None);
}

#[test]
fn the_first_scope_holding_the_device_owns_it() {
    let found = decide(vec![
        (None, Ok(false)),
        (Some("a".into()), Err("down".into())),
        (Some("b".into()), Ok(true)),
    ]);
    assert_eq!(found, Ok(Some(Some("b".to_string()))));
    assert_eq!(decide(vec![(None, Ok(true))]), Ok(Some(None)));
}

#[test]
fn a_failed_lookup_without_a_match_fails_closed() {
    let failed = decide(vec![
        (None, Ok(false)),
        (Some("a".into()), Err("down".into())),
    ]);
    assert_eq!(
        failed,
        Err(OwnerLookupFailed {
            agent: Some("a".to_string()),
            error: "down".to_string(),
        })
    );
    assert_eq!(decide(vec![(None, Ok(false))]), Ok(None));
    assert_eq!(decide(Vec::new()), Ok(None));
}

/// A device revoked in another process leaves its session cipher here; its
/// frames are dropped, not handled as `local`.
#[tokio::test]
async fn a_revoked_device_with_a_live_cipher_is_dropped() {
    let tmp = tempfile::tempdir().unwrap();
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        scoped_config(tmp.path()),
    );
    super::super::rpc::ACTIVE_CIPHERS.lock().unwrap().insert(
        "owner-test-cipher".to_string(),
        std::sync::Arc::new(std::sync::Mutex::new(
            super::super::crypto::TunnelCipher::new(&[7u8; 32]),
        )),
    );
    let owner = crate::core::runtime::CoreContext::scope(
        context.clone(),
        owner_of(
            &DispatchCtx::new(context.clone()),
            "owner-test-cipher",
            None,
        ),
    )
    .await;
    super::super::rpc::ACTIVE_CIPHERS
        .lock()
        .unwrap()
        .remove("owner-test-cipher");
    assert!(owner.is_err(), "{owner:?}");
}

/// A pairing session left over after the handshake does not vouch for a device
/// another process has since revoked, but a device whose row is not persisted
/// yet (the first frame racing the handshake ACK) still belongs to the
/// session's agent.
#[tokio::test]
async fn a_leftover_pairing_session_does_not_outlive_a_revocation() {
    let tmp = tempfile::tempdir().unwrap();
    let config = scoped_config(tmp.path());
    let context = crate::core::runtime::CoreContext::for_test_with_config(
        crate::core::runtime::DomainSet::full(),
        config.clone(),
    );
    let pending = session(None);
    // Before the handshake the session names the agent.
    assert_eq!(
        owner_of(
            &DispatchCtx::process_default(),
            "owner-test-leftover",
            Some(&pending)
        )
        .await,
        Ok(None)
    );
    super::super::rpc::ACTIVE_CIPHERS.lock().unwrap().insert(
        "owner-test-leftover".to_string(),
        std::sync::Arc::new(std::sync::Mutex::new(
            super::super::crypto::TunnelCipher::new(&[9u8; 32]),
        )),
    );
    // Cipher live, row not persisted yet: still the session's agent.
    let racing = crate::core::runtime::CoreContext::scope(
        std::sync::Arc::clone(&context),
        owner_of(
            &DispatchCtx::process_default(),
            "owner-test-leftover",
            Some(&pending),
        ),
    )
    .await;
    assert_eq!(racing, Ok(None));
    // Cipher live, row revoked elsewhere: dropped.
    super::super::store::insert_device(&config, "owner-test-leftover", "label", "pk", "hash")
        .unwrap();
    super::super::store::revoke_device(&config, "owner-test-leftover").unwrap();
    let owner = crate::core::runtime::CoreContext::scope(
        context.clone(),
        owner_of(
            &DispatchCtx::new(context.clone()),
            "owner-test-leftover",
            Some(&pending),
        ),
    )
    .await;
    super::super::rpc::ACTIVE_CIPHERS
        .lock()
        .unwrap()
        .remove("owner-test-leftover");
    assert!(owner.is_err(), "{owner:?}");
}

/// A handshake whose device could not be persisted leaves neither its pairing
/// session nor its cipher behind, so later frames are not accepted.
#[test]
fn an_unpersisted_pairing_is_abandoned() {
    let id = "owner-test-abandon";
    super::super::rpc::PENDING_SESSIONS
        .lock()
        .unwrap()
        .insert(id.to_string(), session(None));
    super::super::rpc::ACTIVE_CIPHERS.lock().unwrap().insert(
        id.to_string(),
        std::sync::Arc::new(std::sync::Mutex::new(
            super::super::crypto::TunnelCipher::new(&[5u8; 32]),
        )),
    );
    super::super::bus::abandon_unpersisted_pairing(id);
    assert!(!super::super::rpc::PENDING_SESSIONS
        .lock()
        .unwrap()
        .contains_key(id));
    assert!(!has_active_cipher(id));
}
