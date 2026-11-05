use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tempfile::TempDir;

// These stores have no TinySecurity config, so any read that reaches the
// keyring fails; a successful read can only come from the cache.
fn cache_secret(store: &AuthProfilesStore, profile_id: &str, token: &str) {
    let name = store.keyring_name(&store.keychain_key_for_profile(profile_id));
    read_cache().secrets.insert(name, secret(token));
}

fn secret(token: &str) -> KeychainSecrets {
    KeychainSecrets {
        token: Some(token.into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn a_secret_read_once_is_served_again_without_the_keyring() {
    let dir = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(dir.path(), false);
    cache_secret(&store, "app-session:default", "synthetic-session");

    let secrets = store
        .keychain_load_secrets("app-session:default")
        .await
        .expect("served from the cache")
        .expect("cached entry");
    assert_eq!(secrets.token.as_deref(), Some("synthetic-session"));
}

#[tokio::test]
async fn another_profile_store_does_not_see_the_cached_secret() {
    let (dir, other) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    cache_secret(
        &AuthProfilesStore::new(dir.path(), false),
        "app-session:default",
        "synthetic-session",
    );

    assert!(AuthProfilesStore::new(other.path(), false)
        .keychain_load_secrets("app-session:default")
        .await
        .is_err());
}

#[tokio::test]
async fn a_delete_or_a_write_forgets_the_cached_secret_even_when_it_fails() {
    let dir = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(dir.path(), false);

    cache_secret(&store, "app-session:default", "synthetic-session");
    assert!(store
        .keychain_delete_secrets("app-session:default")
        .await
        .is_err());
    assert!(store
        .keychain_load_secrets("app-session:default")
        .await
        .is_err());

    cache_secret(&store, "api-key:default", "synthetic-key");
    let profile = AuthProfile::new_token("api-key", "default", "replacement".into());
    assert!(store.keychain_store_secrets(&profile).await.is_err());
    assert!(store
        .keychain_load_secrets("api-key:default")
        .await
        .is_err());
}

#[tokio::test]
async fn stores_for_the_same_keyring_entry_share_its_invalidation() {
    // Two state directories for the same user name the same keyring entry.
    let (a, b) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let first = AuthProfilesStore::new(&a.path().join("user-shared"), false);
    let second = AuthProfilesStore::new(&b.path().join("user-shared"), false);
    cache_secret(&first, "app-session:default", "synthetic-session");

    assert!(second
        .keychain_delete_secrets("app-session:default")
        .await
        .is_err());
    assert!(first
        .keychain_load_secrets("app-session:default")
        .await
        .is_err());
}

#[tokio::test]
async fn concurrent_cold_reads_of_one_entry_ask_the_keyring_once() {
    let name = "user-concurrent:auth-profile:app-session:default";
    let gets = AtomicUsize::new(0);
    let fetch = || async {
        gets.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        Ok(Some(secret("synthetic-session")))
    };

    let (first, second) = tokio::join!(read_through(name, fetch()), read_through(name, fetch()));

    assert_eq!(gets.load(Ordering::SeqCst), 1);
    assert_eq!(
        first.unwrap().unwrap().token.as_deref(),
        Some("synthetic-session")
    );
    assert_eq!(
        second.unwrap().unwrap().token.as_deref(),
        Some("synthetic-session")
    );
}

#[tokio::test]
async fn a_read_overtaken_by_a_write_does_not_cache_what_it_read() {
    let name = "user-overtaken:auth-profile:app-session:default";
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let read = read_through(name, async {
        released.await.unwrap();
        Ok(Some(secret("before-the-write")))
    });
    let write = async {
        tokio::task::yield_now().await;
        // What keychain_store_secrets and keychain_delete_secrets do first.
        read_cache().forget(name);
        release.send(()).unwrap();
    };

    let (read, ()) = tokio::join!(read, write);

    assert_eq!(
        read.unwrap().unwrap().token.as_deref(),
        Some("before-the-write")
    );
    assert!(!read_cache().secrets.contains_key(name));
}

fn held_back(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<KeyringConsentError>(),
        Some(KeyringConsentError::HeldBack)
    )
}

#[tokio::test]
async fn nothing_reaches_the_keyring_while_it_is_held_back() {
    let dir = TempDir::new().unwrap();
    let mut config = crate::credentials::test_support::plaintext_fixture_config(dir.path());
    config.modules.enabled = false;
    let store = AuthProfilesStore::new_with_config(dir.path(), true, &config);

    let read = without_keyring(store.keychain_load_secrets("app-session:default"))
        .await
        .unwrap_err();
    assert!(held_back(&read), "{read:#}");
    // The same read outside the scope does ask, and fails for another reason.
    let asked = store
        .keychain_load_secrets("app-session:default")
        .await
        .unwrap_err();
    assert!(!held_back(&asked), "{asked:#}");
}

#[tokio::test]
async fn a_held_back_write_is_refused_not_moved_into_the_document() {
    let error = anyhow::Error::new(KeyringConsentError::HeldBack);
    assert!(is_consent_error(&error));
    assert!(!should_fallback_to_encrypted_document(&error));
}

#[tokio::test]
async fn a_held_back_read_takes_a_cached_secret_but_never_waits_for_a_cold_one() {
    let name = "user-held-back:auth-profile:app-session:default";
    read_cache()
        .secrets
        .insert(format!("{name}:cached"), secret("synthetic-session"));
    let cached = without_keyring(read_through(&format!("{name}:cached"), async {
        unreachable!("a cached secret is served without the keyring")
    }))
    .await
    .unwrap()
    .unwrap();
    assert_eq!(cached.token.as_deref(), Some("synthetic-session"));

    // Another read holds the entry's gate, as one waiting on a prompt would.
    let gate = Arc::clone(read_cache().gates.entry(name.to_owned()).or_default());
    let _held = gate.lock().await;
    let cold = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        without_keyring(read_through(name, async {
            unreachable!("a held-back read never asks the keyring")
        })),
    )
    .await
    .expect("a held-back read does not wait on the gate")
    .unwrap_err();
    assert!(held_back(&cold), "{cold:#}");
}
