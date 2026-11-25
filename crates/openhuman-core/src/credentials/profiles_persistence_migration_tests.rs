use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn denied_keyring_read_preserves_keyring_only_oauth_profile() {
    let mut profile = PersistedAuthProfile {
        provider: "openai-codex".into(),
        profile_name: "default".into(),
        kind: "oauth".into(),
        ..PersistedAuthProfile::default()
    };
    assert!(
        super::super::migration::preserve_keyring_only_oauth_after_access_failure(&profile, true)
    );

    profile.access_token = Some("enc:not-readable".into());
    assert!(
        !super::super::migration::preserve_keyring_only_oauth_after_access_failure(&profile, true)
    );
    assert!(
        !super::super::migration::preserve_keyring_only_oauth_after_access_failure(&profile, false)
    );
}

#[tokio::test]
async fn auth_profile_kind_serde_roundtrip() {
    let json = serde_json::to_string(&AuthProfileKind::OAuth).unwrap();
    assert_eq!(json, "\"o-auth\""); // kebab-case
    let back: AuthProfileKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, AuthProfileKind::OAuth);

    let json = serde_json::to_string(&AuthProfileKind::Token).unwrap();
    assert_eq!(json, "\"token\"");
}

#[tokio::test]
async fn migration_normalizes_provider_but_preserves_profile_name_casing() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let raw_json = serde_json::json!({
        "schema_version": 1,
        "updated_at": "2026-08-07T10:00:00Z",
        "active_profiles": {
            "provider:DeepSeek": "provider:DeepSeek:Work"
        },
        "profiles": {
            "provider:DeepSeek:Work": {
                "provider": "provider:DeepSeek",
                "profile_name": "Work",
                "kind": "token",
                "token": "sk-test-secret",
                "created_at": "2026-08-07T10:00:00Z",
                "updated_at": "2026-08-07T10:00:00Z"
            }
        }
    });
    std::fs::write(
        store.legacy_path.as_path(),
        serde_json::to_vec_pretty(&raw_json).unwrap(),
    )
    .unwrap();

    let data = store.load().await.unwrap();
    let profile = data.profiles.get("provider:deepseek:Work").unwrap();
    assert_eq!(profile.id, "provider:deepseek:Work");
    assert_eq!(profile.provider, "provider:deepseek");
    assert_eq!(
        data.active_profiles.get("provider:deepseek"),
        Some(&profile.id)
    );
}

#[tokio::test]
async fn migration_collision_prefers_existing_lowercase_key() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let raw_json = serde_json::json!({
        "schema_version": 1,
        "updated_at": "2026-08-07T10:00:00Z",
        "active_profiles": {
            "provider:DeepSeek": "provider:deepseek:legacy",
            "provider:deepseek": "provider:deepseek:canonical"
        },
        "profiles": {
            "provider:deepseek:canonical": {
                "provider": "provider:deepseek",
                "profile_name": "canonical",
                "kind": "token",
                "token": "sk-test-canonical",
                "created_at": "2026-08-07T10:00:00Z",
                "updated_at": "2026-08-07T10:00:00Z"
            },
            "provider:deepseek:legacy": {
                "provider": "provider:deepseek",
                "profile_name": "legacy",
                "kind": "token",
                "token": "sk-test-legacy",
                "created_at": "2026-08-07T10:00:00Z",
                "updated_at": "2026-08-07T10:00:00Z"
            }
        }
    });

    std::fs::write(
        store.legacy_path.as_path(),
        serde_json::to_vec_pretty(&raw_json).unwrap(),
    )
    .unwrap();

    let data = store.load().await.unwrap();

    assert_eq!(
        data.active_profiles.get("provider:deepseek").unwrap(),
        "provider:deepseek:canonical",
        "lowercase key must win over mixed-case collision"
    );
}

#[tokio::test]
async fn migration_profile_id_collision_prefers_canonical_lowercase() {
    // Two profiles whose IDs differ only by casing.
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let raw_json = serde_json::json!({
        "schema_version": 1,
        "updated_at": "2026-08-07T10:00:00Z",
        "active_profiles": {
            "provider:deepseek": "provider:deepseek:default"
        },
        "profiles": {
            "provider:DeepSeek:default": {
                "provider": "provider:DeepSeek",
                "profile_name": "default",
                "kind": "token",
                "token": "sk-mixed-case",
                "created_at": "2026-08-07T10:00:00Z",
                "updated_at": "2026-08-07T10:00:00Z"
            },
            "provider:deepseek:default": {
                "provider": "provider:deepseek",
                "profile_name": "default",
                "kind": "token",
                "token": "sk-canonical",
                "created_at": "2026-08-07T10:00:00Z",
                "updated_at": "2026-08-07T10:00:00Z"
            }
        }
    });

    std::fs::write(
        store.legacy_path.as_path(),
        serde_json::to_vec_pretty(&raw_json).unwrap(),
    )
    .unwrap();

    let data = store.load().await.unwrap();

    assert_eq!(data.profiles.len(), 1);
    let profile = data.profiles.get("provider:deepseek:default").unwrap();
    assert_eq!(profile.token.as_deref(), Some("sk-canonical"));
}
