use super::*;
use tempfile::TempDir;

// These stores have no TinySecurity config, so any read that reaches the
// keyring fails; a successful read can only come from the cache.
fn cache_secret(store: &AuthProfilesStore, profile_id: &str, token: &str) {
    let name = store.keyring_name(&store.keychain_key_for_profile(profile_id));
    read_cache().insert(
        store.read_cache_key(&name),
        KeychainSecrets {
            token: Some(token.into()),
            ..Default::default()
        },
    );
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
