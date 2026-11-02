use super::*;

#[tokio::test]
async fn persisted_profile_document_is_encrypted_at_rest() {
    let tmp = tempfile::TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);
    store
        .upsert_profile(
            AuthProfile::new_token("openai", "default", "profile-secret-token".into()),
            true,
        )
        .await
        .unwrap();

    let bytes = std::fs::read(store.path()).expect("encrypted profile document");
    assert!(!String::from_utf8_lossy(&bytes).contains("profile-secret-token"));
    assert!(serde_json::from_slice::<PersistedAuthProfiles>(&bytes).is_err());
    assert_eq!(
        store.load().await.unwrap().profiles["openai:default"]
            .token
            .as_deref(),
        Some("profile-secret-token")
    );
}

#[tokio::test]
async fn configured_generic_storage_does_not_replace_the_profile_local_document() {
    let _backend = crate::storage::test_support::install_backend_for_test(
        crate::storage::test_support::memory_backend(),
    );
    let tmp = tempfile::TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);
    store
        .upsert_profile(
            AuthProfile::new_token("openai", "default", "local-secret".into()),
            true,
        )
        .await
        .unwrap();

    let bytes = std::fs::read(store.path()).expect("profile-local encrypted document");
    assert!(!String::from_utf8_lossy(&bytes).contains("local-secret"));
    assert_eq!(
        store.load().await.unwrap().profiles["openai:default"]
            .token
            .as_deref(),
        Some("local-secret")
    );
}
