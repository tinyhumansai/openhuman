use super::*;
use std::collections::BTreeMap;
use tempfile::TempDir;

#[tokio::test]
#[cfg(feature = "security-module")]
async fn read_only_profile_loads_secrets_stored_in_tinysecurity_keyring() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path();
    let config = config_for(state_dir);

    let id = "openai-codex:default";
    let mut persisted = PersistedAuthProfiles::default();
    persisted.profiles.insert(
        id.into(),
        PersistedAuthProfile {
            provider: "openai-codex".into(),
            profile_name: "default".into(),
            kind: "oauth".into(),
            ..PersistedAuthProfile::default()
        },
    );
    persisted.active_profiles = BTreeMap::from([("openai-codex".into(), id.into())]);
    write_profile_document(state_dir, &persisted);

    let loaded = load_profile_with_keyring(&config, "openai-codex", None, |_, _| {
        Box::pin(async {
            Ok(Some(KeychainSecrets {
                access_token: Some("keyring-access-token".into()),
                ..KeychainSecrets::default()
            }))
        })
    })
    .await
    .expect("keyring-backed profile should be readable")
    .expect("profile should be selected");

    assert_eq!(loaded.id, id);
    assert_eq!(
        loaded.token_set.unwrap().access_token,
        "keyring-access-token"
    );
}

#[cfg(feature = "security-module")]
#[tokio::test]
async fn read_only_profile_does_not_bypass_denied_keyring_consent() {
    let tmp = TempDir::new().unwrap();
    let state_dir = tmp.path();
    let config = config_for(state_dir);
    let id = "openai-codex:default";
    let mut persisted = PersistedAuthProfiles::default();
    persisted.profiles.insert(
        id.into(),
        PersistedAuthProfile {
            provider: "openai-codex".into(),
            profile_name: "default".into(),
            kind: "oauth".into(),
            // This encrypted-document copy must not be read after denial.
            access_token: Some("document-fallback-token".into()),
            ..PersistedAuthProfile::default()
        },
    );
    persisted.active_profiles = BTreeMap::from([("openai-codex".into(), id.into())]);
    write_profile_document(state_dir, &persisted);

    let result = load_profile_with_keyring(&config, "openai-codex", None, |_, _| {
        Box::pin(async {
            Err(anyhow::Error::new(
                super::super::keychain::KeyringConsentError::Denied,
            ))
        })
    })
    .await;

    assert!(matches!(result, Err(ReadOnlyAuthProfileError::Unavailable)));
}

fn config_for(state_dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = state_dir.join("config.toml");
    config.workspace_dir = state_dir.to_path_buf();
    config
}

fn write_profile_document(state_dir: &std::path::Path, persisted: &PersistedAuthProfiles) {
    let document = serde_json::to_vec(persisted).unwrap();
    let path = super::super::profiles_file(state_dir);
    let directory = path.parent().unwrap();
    std::fs::create_dir_all(directory).unwrap();
    let store = EncryptedFileSecrets::with_key_file(directory).unwrap();
    crate::storage::block_on(async move {
        store
            .set(super::super::PROFILE_DOCUMENT_SECRET_NAME, &document)
            .await
    })
    .unwrap();
}

#[tokio::test]
async fn explicit_document_credentials_do_not_consult_a_stale_keyring_copy() {
    let workspace = TempDir::new().unwrap();
    let config = crate::credentials::test_support::plaintext_fixture_config(workspace.path());
    let document = AuthProfilesStore::new(workspace.path(), false);
    let profile = AuthProfile::new_token("openai", "default", "document-owned-token".into());
    document.upsert_profile(profile, true).await.unwrap();
    let loaded = load_profile_with_keyring(&config, "openai", None, |_, _| {
        Box::pin(async { panic!("an explicit document credential has no keyring read authority") })
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(loaded.token.as_deref(), Some("document-owned-token"));
}

#[tokio::test]
async fn a_keyring_owned_record_cannot_supply_a_document_secret_as_fallback() {
    let workspace = TempDir::new().unwrap();
    let config = crate::credentials::test_support::plaintext_fixture_config(workspace.path());
    let mut persisted = PersistedAuthProfiles::default();
    persisted.profiles.insert(
        "openai:default".into(),
        PersistedAuthProfile {
            provider: "openai".into(),
            profile_name: "default".into(),
            kind: "token".into(),
            secret_location: Some(super::super::persistence::SecretLocation::Keyring),
            token: Some("stale-document-copy".into()),
            ..Default::default()
        },
    );
    write_profile_document(workspace.path(), &persisted);
    let result = load_profile_with_keyring(&config, "openai", None, |_, _| {
        Box::pin(async {
            panic!("mixed credential ownership must fail before consulting a backend")
        })
    })
    .await;
    assert_eq!(result.unwrap_err(), ReadOnlyAuthProfileError::InvalidStore);
}

fn file_tree(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn visit(root: &std::path::Path, at: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(at).expect("read fixture tree") {
            let path = entry.expect("read fixture entry").path();
            out.push(
                path.strip_prefix(root)
                    .expect("relative path")
                    .to_path_buf(),
            );
            if path.is_dir() {
                visit(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(root, root, &mut out);
    out.sort();
    out
}

#[tokio::test]
async fn read_only_lookup_of_a_plaintext_session_creates_no_storage() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    // Production opens an on-disk default store; the unit-test default is
    // in-memory and would hide one being created.
    crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
    let doc = serde_json::json!({
        "schema_version": 1,
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": "plaintext-session",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "metadata": {},
            }
        }
    });
    std::fs::write(
        tmp.path().join(PROFILES_FILENAME),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
    let before = file_tree(tmp.path());

    let profile = load_profile(&config, "app-session", None)
        .await
        .expect("a plaintext legacy session is readable without a store")
        .expect("profile present");

    assert_eq!(profile.token.as_deref(), Some("plaintext-session"));
    assert!(
        !config.workspace_dir.join("storage").exists(),
        "a read-only profile lookup must not create the workspace store"
    );
    assert_eq!(
        file_tree(tmp.path()),
        before,
        "a read-only lookup must not write"
    );
}

#[tokio::test]
async fn read_only_lookup_without_a_store_fails_closed_for_a_secret_only_in_the_keyring() {
    let tmp = TempDir::new().unwrap();
    let mut config = Config::default();
    config.config_path = tmp.path().join("config.toml");
    config.workspace_dir = tmp.path().join("workspace");
    config.action_dir = config.workspace_dir.clone();
    crate::storage::local::use_persistent_default_for_test(&config.workspace_dir);
    // A legacy record with no document secret: its token can only be in the
    // keyring, so an unreachable keyring must not read as "no token".
    let doc = serde_json::json!({
        "schema_version": 1,
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z",
                "metadata": {},
            }
        }
    });
    std::fs::write(
        tmp.path().join(PROFILES_FILENAME),
        serde_json::to_vec(&doc).unwrap(),
    )
    .unwrap();
    let before = file_tree(tmp.path());

    let result = load_profile(&config, "app-session", None).await;

    assert!(
        matches!(result, Err(ReadOnlyAuthProfileError::Unavailable)),
        "an unreachable keyring must be reported unavailable, got {result:?}"
    );
    assert_eq!(
        file_tree(tmp.path()),
        before,
        "a read-only lookup must not write"
    );
}
