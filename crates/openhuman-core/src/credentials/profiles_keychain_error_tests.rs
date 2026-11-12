//! Profile keyring configuration stays on the TinySecurity path.

use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn denied_keyring_write_never_falls_back_to_document_secrets() {
    for denial in [
        super::super::keychain::KeyringConsentError::Denied,
        super::super::keychain::KeyringConsentError::Pending,
    ] {
        let error = anyhow::Error::new(denial);
        assert!(!super::super::keychain::should_fallback_to_encrypted_document(&error));
    }
    let unavailable = anyhow::anyhow!("module unavailable");
    assert!(super::super::keychain::should_fallback_to_encrypted_document(&unavailable));
}

#[tokio::test]
async fn no_config_profile_store_uses_its_encrypted_document_without_keyring_bypass() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);

    assert!(!store.use_keychain);
    assert!(store.security_config.is_none());
}

#[tokio::test]
async fn config_backed_profile_store_uses_tinysecurity_and_existing_namespaced_key() {
    let _env =
        crate::config::test_env::EnvVarGuard::locked_unset_async("OPENHUMAN_KEYRING_BACKEND").await;
    let tmp = TempDir::new().unwrap();
    let config = crate::config::Config::default();
    let store = AuthProfilesStore::new_with_config(tmp.path(), true, &config);

    assert_eq!(store.use_keychain, cfg!(feature = "security-module"));
    assert!(store.security_config.is_some());
    assert_eq!(
        super::super::namespaced_keyring_name("user-123", "auth:openai:default"),
        "user-123:auth:openai:default"
    );
}

/// An operator who picks a file keyring (a headless core: no OS keyring and
/// no one to answer TinySecurity's per-operation prompt) keeps profile
/// secrets in the encrypted profile document. Asking the keyring instead
/// waited out the 120 s prompt and refused the write.
#[tokio::test]
async fn file_keyring_backend_stores_profile_secrets_without_asking_the_keyring() {
    for backend in ["file", "encrypted_file", "memory"] {
        let _env = crate::config::test_env::EnvVarGuard::locked_set_async(
            "OPENHUMAN_KEYRING_BACKEND",
            backend,
        )
        .await;
        let tmp = TempDir::new().unwrap();
        let config = crate::config::Config::default();
        let store = AuthProfilesStore::new_with_config(tmp.path(), false, &config);
        let profile = AuthProfile::new_token("memory-cortexdb", "default", "ck-test".into());

        // Inside `without_keyring` any keyring call is refused like an
        // unanswered prompt, so this write succeeds only if none is made.
        super::super::keychain::without_keyring(store.upsert_profile(profile, true))
            .await
            .unwrap_or_else(|e| panic!("backend={backend}: {e:#}"));

        assert!(!store.use_keychain, "backend={backend}");
        let loaded = store.load().await.unwrap();
        assert_eq!(
            loaded.profiles["memory-cortexdb:default"].token.as_deref(),
            Some("ck-test"),
            "backend={backend}"
        );
    }
}

#[tokio::test]
async fn unavailable_keyring_keeps_keyring_only_oauth_metadata_in_place() {
    let tmp = TempDir::new().unwrap();
    let config = crate::config::Config::default();
    let store = AuthProfilesStore::new_with_config(tmp.path(), true, &config);
    let profile = AuthProfile::new_oauth(
        "openai-codex",
        "default",
        TokenSet {
            access_token: "not-persisted-here".into(),
            refresh_token: None,
            id_token: None,
            expires_at: None,
            token_type: None,
            scope: None,
        },
    );
    let id = profile.id.clone();
    let persisted = PersistedAuthProfile {
        provider: profile.provider,
        profile_name: profile.profile_name,
        kind: "oauth".into(),
        ..PersistedAuthProfile::default()
    };
    let mut document = PersistedAuthProfiles::default();
    document.profiles.insert(id.clone(), persisted);
    store.write_persisted_locked(&document).unwrap();

    let loaded = store
        .load()
        .await
        .expect("unavailable keyring is handled safely");
    assert!(!loaded.profiles.contains_key(&id));
    assert!(store
        .read_persisted_locked()
        .unwrap()
        .profiles
        .contains_key(&id));
}

#[tokio::test]
async fn denied_keyring_delete_restores_profile_document() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);
    let mut before = PersistedAuthProfiles::default();
    before.profiles.insert(
        "openai:default".into(),
        PersistedAuthProfile {
            provider: "openai".into(),
            profile_name: "default".into(),
            kind: "oauth".into(),
            ..PersistedAuthProfile::default()
        },
    );
    store
        .write_persisted_locked(&PersistedAuthProfiles::default())
        .unwrap();

    let error = store
        .rollback_profile_removal(
            "openai:default",
            &before,
            anyhow::Error::new(super::super::keychain::KeyringConsentError::Denied),
        )
        .expect_err("a denied keyring delete must keep removal from succeeding");

    assert!(error
        .to_string()
        .contains("profile removal was not completed"));
    assert_eq!(store.read_persisted_locked().unwrap().profiles.len(), 1);
    assert!(store
        .read_persisted_locked()
        .unwrap()
        .profiles
        .contains_key("openai:default"));
}

#[tokio::test]
async fn a_document_only_store_cannot_remove_a_keyring_owned_record() {
    let workspace = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(workspace.path(), false);
    let mut persisted = PersistedAuthProfiles::default();
    persisted.profiles.insert(
        "openai:default".into(),
        PersistedAuthProfile {
            provider: "openai".into(),
            profile_name: "default".into(),
            kind: "token".into(),
            secret_location: Some(super::super::persistence::SecretLocation::Keyring),
            ..Default::default()
        },
    );
    store.write_persisted_locked(&persisted).unwrap();
    assert!(store.remove_profile("openai:default").await.is_err());
    assert!(store
        .read_persisted_locked()
        .unwrap()
        .profiles
        .contains_key("openai:default"));
}

#[tokio::test]
async fn an_unavailable_keyring_record_survives_an_unrelated_document_write() {
    let workspace = TempDir::new().unwrap();
    let mut config = crate::credentials::test_support::plaintext_fixture_config(workspace.path());
    config.modules.enabled = false;
    let store = AuthProfilesStore::new_with_config(workspace.path(), false, &config);
    let mut persisted = PersistedAuthProfiles::default();
    persisted.profiles.insert(
        "openai:default".into(),
        PersistedAuthProfile {
            provider: "openai".into(),
            profile_name: "default".into(),
            kind: "token".into(),
            secret_location: Some(super::super::persistence::SecretLocation::Keyring),
            ..Default::default()
        },
    );
    store.write_persisted_locked(&persisted).unwrap();
    let before = serde_json::to_value(&persisted.profiles["openai:default"]).unwrap();
    store
        .upsert_profile(
            AuthProfile::new_token("other", "default", "new-synthetic-token".into()),
            true,
        )
        .await
        .unwrap();
    let current = store.read_persisted_locked().unwrap();
    assert_eq!(
        serde_json::to_value(&current.profiles["openai:default"]).unwrap(),
        before
    );
    assert!(current.profiles.contains_key("other:default"));
}

#[tokio::test]
async fn denied_keyring_removal_rollback_preserves_other_profile_updates() {
    let workspace = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(workspace.path(), false);
    let mut before = PersistedAuthProfiles::default();
    before.profiles.insert(
        "openai:default".into(),
        PersistedAuthProfile {
            provider: "openai".into(),
            profile_name: "default".into(),
            kind: "token".into(),
            ..Default::default()
        },
    );
    let mut current = PersistedAuthProfiles::default();
    current.profiles.insert(
        "other:default".into(),
        PersistedAuthProfile {
            provider: "other".into(),
            profile_name: "default".into(),
            kind: "token".into(),
            token: Some("concurrently-updated-value".into()),
            ..Default::default()
        },
    );
    store.write_persisted_locked(&current).unwrap();
    assert!(store
        .rollback_profile_removal(
            "openai:default",
            &before,
            anyhow::Error::new(super::super::keychain::KeyringConsentError::Denied)
        )
        .is_err());
    let restored = store.read_persisted_locked().unwrap();
    assert!(restored.profiles.contains_key("openai:default"));
    assert_eq!(
        serde_json::to_value(&restored.profiles["other:default"]).unwrap(),
        serde_json::to_value(&current.profiles["other:default"]).unwrap()
    );
}
