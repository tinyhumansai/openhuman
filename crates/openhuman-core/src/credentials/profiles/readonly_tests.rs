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
        Ok(Some(KeychainSecrets {
            access_token: Some("keyring-access-token".into()),
            ..KeychainSecrets::default()
        }))
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
        Err(anyhow::Error::new(
            super::super::keychain::KeyringConsentError::Denied,
        ))
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
