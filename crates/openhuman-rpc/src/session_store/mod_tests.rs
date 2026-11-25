use super::*;
use tinyagents_session::turn_state::TurnState;

/// The provider and storage slots are process-wide; these tests take turns
/// with each other and with any test that stores a credential.
use crate::STORAGE_SLOT_TEST_LOCK as SLOTS;

/// Puts the process-global storage backend back as a test found it.
fn restore_backend(previous: Option<Arc<dyn crate::core_host::storage::StorageBackend>>) {
    match previous {
        Some(backend) => {
            crate::core_host::storage::install(backend);
        }
        None => {
            crate::core_host::storage::clear();
        }
    }
}

#[tokio::test]
async fn a_storage_url_installs_the_driver_backed_store() {
    let _turn = SLOTS.lock().await;
    let previous = crate::core_host::agent::session_store::installed();
    let previous_backend = crate::core_host::storage::installed();
    install_for_url(Some("memory".into())).await.unwrap();

    let provider = crate::core_host::agent::session_store::installed().unwrap();
    assert!(
        provider
            .destination_key()
            .is_some_and(|key| !key.is_empty()),
        "{:?}",
        provider.destination_key()
    );
    assert!(crate::core_host::storage::installed().is_some());
    let installed_url = crate::core_host::storage::installed_url().unwrap();
    assert_eq!(
        crate::core_host::storage::StorageUrl::parse(&installed_url)
            .unwrap()
            .driver(),
        "memory"
    );
    // Agents are kept apart in the shared backend.
    let alice = provider.for_agent("alice");
    alice
        .turn_states
        .put(&TurnState::started("t", "r", 8, "2026-01-01T00:00:00Z"))
        .unwrap();
    assert!(provider
        .for_agent("bob")
        .turn_states
        .get("t")
        .unwrap()
        .is_none());

    restore_backend(previous_backend);
    crate::core_host::agent::session_store::restore(previous);
}

/// A storage URL installs the driver-backed store, and two agents on the one
/// backend keep their turn states apart. Run once per driver the build has.
#[cfg(any(
    feature = "storage-sqlite",
    feature = "storage-file",
    feature = "storage-mongodb"
))]
async fn driver_backed_store_keeps_agents_apart(url: String) {
    let _turn = SLOTS.lock().await;
    let previous = crate::core_host::agent::session_store::installed();
    let previous_backend = crate::core_host::storage::installed();
    install_for_url(Some(url.clone())).await.unwrap();

    assert!(crate::core_host::storage::installed().is_some());
    assert_eq!(
        crate::core_host::storage::installed_url().as_deref(),
        Some(url.as_str())
    );
    let provider = crate::core_host::agent::session_store::installed().unwrap();
    let alice = provider.for_agent("alice");
    alice
        .turn_states
        .put(&TurnState::started("t", "r", 8, "2026-01-01T00:00:00Z"))
        .unwrap();
    assert!(alice.turn_states.get("t").unwrap().is_some());
    assert!(provider
        .for_agent("bob")
        .turn_states
        .get("t")
        .unwrap()
        .is_none());

    restore_backend(previous_backend);
    crate::core_host::agent::session_store::restore(previous);
}

#[cfg(feature = "storage-sqlite")]
#[tokio::test(flavor = "multi_thread")]
async fn the_sqlite_backend_keeps_agents_apart() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("sessions").display());
    driver_backed_store_keeps_agents_apart(url).await;
}

#[tokio::test]
async fn an_unusable_url_fails_the_boot() {
    let _turn = SLOTS.lock().await;
    let previous = crate::core_host::agent::session_store::installed();
    let error = install_for_url(Some("ftp://nowhere".into()))
        .await
        .unwrap_err();
    assert!(format!("{error:#}").contains("storage"), "{error:#}");
    crate::core_host::agent::session_store::restore(previous);
}

#[tokio::test]
async fn the_host_reads_the_storage_url_from_the_environment() {
    let _turn = SLOTS.lock().await;
    let previous = crate::core_host::agent::session_store::installed();
    let previous_backend = crate::core_host::storage::installed();
    let var = crate::core_host::storage::STORAGE_URL_VAR;
    let old = std::env::var_os(var);

    std::env::set_var(var, " memory ");
    let booted = install_for_host().await;
    let installed_url = crate::core_host::storage::installed_url();

    std::env::set_var(var, "ftp://nowhere");
    let refused = install_for_host().await;

    match old {
        Some(value) => std::env::set_var(var, value),
        None => std::env::remove_var(var),
    }
    restore_backend(previous_backend);
    crate::core_host::agent::session_store::restore(previous);

    booted.unwrap();
    assert_eq!(
        installed_url
            .as_deref()
            .and_then(|url| crate::core_host::storage::StorageUrl::parse(url).ok())
            .map(|url| url.driver()),
        Some("memory")
    );
    assert!(refused.is_err(), "an unusable env URL fails the boot");
}

#[tokio::test]
async fn provider_for_url_returns_the_store_without_installing_it() {
    let _turn = SLOTS.lock().await;
    let previous = crate::core_host::agent::session_store::installed();
    let previous_backend = crate::core_host::storage::installed();
    crate::core_host::agent::session_store::restore(None);

    let backed = provider_for_url(Some("memory".into())).await.unwrap();
    let backed_key = backed.destination_key();
    let installed_url = crate::core_host::storage::installed_url();
    let left_uninstalled = crate::core_host::agent::session_store::installed().is_none();

    // No URL: each workspace keeps its sessions in its own default database.
    let per_workspace = provider_for_url(None).await.unwrap();
    let cleared = crate::core_host::storage::installed().is_none();

    restore_backend(previous_backend);
    crate::core_host::agent::session_store::restore(previous);

    assert!(
        backed_key.as_deref().is_some_and(|key| !key.is_empty()),
        "{backed_key:?}"
    );
    assert_eq!(
        installed_url
            .as_deref()
            .and_then(|url| crate::core_host::storage::StorageUrl::parse(url).ok())
            .map(|url| url.driver()),
        Some("memory"),
        "the memory backend is installed"
    );
    assert!(
        left_uninstalled,
        "the provider is handed back, not installed"
    );
    drop(per_workspace);
    assert!(cleared, "no URL clears an earlier backend");
}

#[tokio::test]
async fn provider_for_host_reads_the_storage_url_from_the_environment() {
    let _turn = SLOTS.lock().await;
    let previous_backend = crate::core_host::storage::installed();
    let var = crate::core_host::storage::STORAGE_URL_VAR;
    let old = std::env::var_os(var);

    std::env::set_var(var, "memory");
    let booted = provider_for_host().await.map(|p| p.destination_key());
    std::env::set_var(var, "ftp://nowhere");
    let refused = provider_for_host().await;

    match old {
        Some(value) => std::env::set_var(var, value),
        None => std::env::remove_var(var),
    }
    restore_backend(previous_backend);

    let key = booted.unwrap();
    assert!(key.as_deref().is_some_and(|key| !key.is_empty()), "{key:?}");
    assert!(refused.is_err(), "an unusable env URL fails the TUI boot");
}
