use super::*;
use crate::config::ComposioHostCredential;
use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
use crate::tools::toolpacks::ToolGroups;
use std::sync::Arc;

#[derive(Clone)]
struct CapturedScopeLogs(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CapturedScopeLogs {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn dormant_scope_epochs_do_not_accumulate_with_profile_churn() {
    let _guard = composio_cache_test_lock();
    for index in 0..128 {
        drop(cache_generation(&format!(
            "review-dormant-{index}/snapshot"
        )));
    }
    let count = CACHE_GENERATIONS
        .read()
        .unwrap()
        .keys()
        .filter(|key| key.starts_with("review-dormant-"))
        .count();
    assert!(
        count <= 1,
        "dormant epochs must not grow with historical profiles: {count}"
    );
}

#[test]
fn epoch_reclamation_preserves_the_lease_fencing_an_in_flight_fetch() {
    let _guard = composio_cache_test_lock();
    let original = cache_generation("review-live/snapshot");
    original.fetch_add(1, Ordering::SeqCst);
    for index in 0..128 {
        drop(cache_generation(&format!("review-churn-{index}/snapshot")));
    }
    let resumed = cache_generation("review-live/snapshot");
    assert!(Arc::ptr_eq(&original, &resumed));
    assert_eq!(resumed.load(Ordering::SeqCst), 1);
}

/// Give both profiles one isolated configuration and one fake service identity.
fn config(dir: &std::path::Path) -> Config {
    let mut config = Config {
        workspace_dir: dir.join("workspace"),
        action_dir: dir.join("workspace"),
        config_path: dir.join("config.toml"),
        ..Config::default()
    };
    // A fixed fake host credential makes the shared service identity explicit.
    // These tests seed cached data and never call the backend.
    config.composio.pin_host_credential(
        ComposioHostCredential::direct("ck_shared_scope_fixture")
            .entity_id("shared-service-entity"),
    );
    config
}

/// Derive a real profile context while keeping the shared configuration intact.
fn profile(config: &Config, id: &str) -> Arc<CoreContext> {
    CoreContext::for_test_with_config(DomainSet::kernel(), config.clone()).derive_with(
        ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none()).profile(id),
    )
}
/// Install a known snapshot under the supplied context's production cache key.
fn seed(config: &Config, ctx: Arc<CoreContext>, toolkit: &str) -> String {
    CoreContext::sync_scope(ctx, || {
        let key = cache_key(config);
        INTEGRATIONS_CACHE.write().unwrap().insert(
            key.clone(),
            CachedIntegrations {
                entries: vec![ConnectedIntegration {
                    toolkit: toolkit.into(),
                    description: String::new(),
                    tools: vec![],
                    gated_tools: vec![],
                    connected: true,
                    connections: vec![],
                    non_active_status: None,
                }],
                cached_at: Instant::now(),
            },
        );
        key
    })
}

/// Matching credentials must not let one profile read another's snapshot.
#[test]
fn profiles_with_identical_credentials_cannot_read_each_others_integrations() {
    let _guard = composio_cache_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let alice = profile(&config, "isolation-alice");
    let bob = profile(&config, "isolation-bob");
    let key = seed(&config, alice.clone(), "gmail");
    CoreContext::sync_scope(bob.clone(), || {
        assert!(
            cached_active_integrations(&config).is_none(),
            "identical service credentials must not grant another profile's cached connections"
        );
    });
    let bob_key = seed(&config, bob, "slack");
    assert_ne!(
        key, bob_key,
        "profile identity must participate in the cache key"
    );
    CoreContext::sync_scope(alice, || {
        assert_eq!(
            cached_active_integrations(&config).unwrap()[0].toolkit,
            "gmail"
        );
    });
    let mut guard = INTEGRATIONS_CACHE.write().unwrap();
    guard.remove(&key);
    guard.remove(&bob_key);
}

/// Reconcile only the caller's entries against its connection response.
#[test]
fn reconciliation_for_one_profile_preserves_another_profiles_connections() {
    let _guard = composio_cache_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let alice = profile(&config, "reconciliation-alice");
    let bob = profile(&config, "reconciliation-bob");
    let alice_key = seed(&config, alice.clone(), "gmail");
    let bob_key = seed(&config, bob.clone(), "slack");
    assert!(CoreContext::sync_scope(
        bob,
        || sync_cache_with_connections(&[])
    ));
    CoreContext::sync_scope(alice, || {
        assert_eq!(
            cached_active_integrations(&config)
                .expect("another profile's connection list must preserve this snapshot")[0]
                .toolkit,
            "gmail"
        );
    });
    let mut guard = INTEGRATIONS_CACHE.write().unwrap();
    assert!(!guard.contains_key(&bob_key));
    guard.remove(&alice_key);
}
/// Invalidation must leave another profile's cached snapshot readable.
#[test]
fn profile_invalidation_preserves_another_profiles_snapshot() {
    let _guard = composio_cache_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let alice = profile(&config, "invalidation-alice");
    let bob = profile(&config, "invalidation-bob");
    let alice_key = seed(&config, alice.clone(), "gmail");
    let bob_key = seed(&config, bob.clone(), "slack");
    CoreContext::sync_scope(bob, invalidate_connected_integrations_cache);
    assert!(
        INTEGRATIONS_CACHE.read().unwrap().contains_key(&alice_key),
        "Bob's invalidation must preserve Alice's snapshot"
    );
    assert!(!INTEGRATIONS_CACHE.read().unwrap().contains_key(&bob_key));
    CoreContext::sync_scope(alice, || {
        assert_eq!(
            cached_active_integrations(&config).unwrap()[0].toolkit,
            "gmail"
        )
    });
    INTEGRATIONS_CACHE.write().unwrap().remove(&alice_key);
}

/// Fence the caller's pending warm while admitting another profile's result.
#[test]
fn profile_invalidation_does_not_cancel_another_profiles_in_flight_warm() {
    let _guard = composio_cache_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let alice = profile(&config, "warm-alice");
    let bob = profile(&config, "warm-bob");
    let alice_key = CoreContext::sync_scope(alice, || cache_key(&config));
    let bob_key = CoreContext::sync_scope(bob.clone(), || cache_key(&config));
    let alice_epoch = cache_generation(&alice_key);
    let bob_epoch = cache_generation(&bob_key);
    let alice_generation = alice_epoch.load(Ordering::SeqCst);
    let bob_generation = bob_epoch.load(Ordering::SeqCst);
    CoreContext::sync_scope(bob, invalidate_connected_integrations_cache);
    assert_eq!(
        alice_epoch.load(Ordering::SeqCst),
        alice_generation,
        "Alice's in-flight fetch must remain admissible"
    );
    assert_ne!(
        bob_epoch.load(Ordering::SeqCst),
        bob_generation,
        "Bob's own in-flight fetch must be fenced even with no cached entries"
    );
    assert!(
        store_fetched_integrations(alice_key.clone(), &alice_epoch, alice_generation, vec![]),
        "Alice's completed warm must still be admitted through the production insertion path"
    );
    assert!(
        !store_fetched_integrations(bob_key.clone(), &bob_epoch, bob_generation, vec![]),
        "Bob's invalidated warm must never repopulate the cache"
    );
    let mut guard = INTEGRATIONS_CACHE.write().unwrap();
    assert!(guard.remove(&alice_key).is_some());
    assert!(!guard.contains_key(&bob_key));
}

/// Missing SaaS authority must refuse cache reads, fetches and mutations.
#[test]
fn no_saas_profile_cannot_invalidate_or_reconcile_any_snapshot() {
    const CHILD: &str = "OPENHUMAN_TEST_UNSCOPED_CACHE_MUTATION_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "integrations::composio::connected_integrations::cache::scope_tests::no_saas_profile_cannot_invalidate_or_reconcile_any_snapshot", "--nocapture"])
            .env(CHILD, "1").status().unwrap();
        assert!(
            status.success(),
            "isolated SaaS cache mutation regression failed"
        );
        return;
    }
    crate::core::runtime::mode::lock_mode(crate::core::runtime::Mode::Saas).unwrap();
    let captured = CapturedScopeLogs(Arc::new(std::sync::Mutex::new(Vec::new())));
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(move || writer.clone())
        .finish();
    let _subscriber_guard = tracing::subscriber::set_default(subscriber);
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.into();
    config.composio.pin_host_credential(
        ComposioHostCredential::direct("ck_shared_scope_fixture")
            .entity_id("shared-service-entity")
            .base_urls("http://127.0.0.1:1/v2", "http://127.0.0.1:1/v3"),
    );
    let alice = profile(&config, "unscoped-mutation-alice");
    let key = seed(&config, alice, "gmail");
    let epoch = cache_generation(&key);
    let generation = epoch.load(Ordering::SeqCst);
    invalidate_connected_integrations_cache();
    assert!(
        INTEGRATIONS_CACHE.read().unwrap().contains_key(&key),
        "absent profile must not clear any cache"
    );
    assert!(
        !sync_cache_with_connections(&[]),
        "absent profile must not reconcile any cache"
    );
    assert!(INTEGRATIONS_CACHE.read().unwrap().contains_key(&key));
    assert_eq!(
        epoch.load(Ordering::SeqCst),
        generation,
        "absent profile must not fence another tenant's warm"
    );

    assert!(cached_active_integrations(&config).is_none());
    let mut disabled = config.clone();
    disabled.composio.mode = crate::config::schema::COMPOSIO_MODE_DISABLED.into();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // Use the exact enabled configuration Alice's seeded snapshot uses. Even
    // a broken scope check must not send this fixture to a third-party service.
    assert!(matches!(
        runtime.block_on(super::super::fetch::fetch_connected_integrations_status(
            &config
        )),
        super::super::fetch::FetchConnectedIntegrationsStatus::Unavailable
    ));
    assert!(matches!(
        runtime.block_on(super::super::fetch::fetch_connected_integrations_status(
            &disabled
        )),
        super::super::fetch::FetchConnectedIntegrationsStatus::Unavailable
    ));
    let unprofiled = CoreContext::for_test_with_config(DomainSet::kernel(), config.clone());
    CoreContext::sync_scope(unprofiled, || {
        assert!(checked_cache_key(&config).is_none());
        assert!(cached_active_integrations(&config).is_none());
        invalidate_connected_integrations_cache();
        assert!(!sync_cache_with_connections(&[]));
    });
    assert!(INTEGRATIONS_CACHE.read().unwrap().contains_key(&key));
    assert_eq!(epoch.load(Ordering::SeqCst), generation);
    let logs = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
    assert!(
        logs.matches("scope_resolution_refused").count() >= 3,
        "scope failures must be observable for fetch and cache mutation: {logs}"
    );
    assert!(!logs.contains("ck_shared_scope_fixture"));
    assert!(!logs.contains("unscoped-mutation-alice"));
}

/// Desktop agents share their operator catalog and its invalidation epoch.
#[test]
fn desktop_agents_share_catalog_and_default_context_invalidates_their_warms() {
    let _guard = composio_cache_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let config = config(dir.path());
    let root = CoreContext::for_test_with_config(DomainSet::kernel(), config.clone());
    let agent = |id: &str| {
        let mut overlay =
            ContextOverlay::new(config.clone(), DomainSet::kernel(), ToolGroups::none());
        overlay.session_agent = Some(id.into());
        root.derive_with(overlay)
    };
    let first = agent("desktop-first");
    let second = agent("desktop-second");
    let key = seed(&config, first, "gmail");
    let other_profile = profile(&config, "desktop-other-profile");
    let other_key = seed(&config, other_profile, "slack");
    let epoch = cache_generation(&key);
    let generation = epoch.load(Ordering::SeqCst);
    CoreContext::sync_scope(second, || {
        assert_eq!(
            cache_key(&config),
            key,
            "the connected-account catalog belongs to the operator, shared by their agents"
        );
        assert_eq!(
            cached_active_integrations(&config).unwrap()[0].toolkit,
            "gmail"
        );
    });
    CoreContext::sync_scope(root, invalidate_connected_integrations_cache);
    assert!(
        !INTEGRATIONS_CACHE.read().unwrap().contains_key(&key),
        "the desktop settings/default context must invalidate its agents' catalog"
    );
    assert_ne!(
        epoch.load(Ordering::SeqCst),
        generation,
        "the desktop settings/default context must fence its agents' in-flight warms"
    );
    assert!(!store_fetched_integrations(key, &epoch, generation, vec![]));
    let mut guard = INTEGRATIONS_CACHE.write().unwrap();
    assert!(
        guard.remove(&other_key).is_some(),
        "desktop invalidation must preserve explicit profiles"
    );
}
