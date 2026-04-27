use super::*;
use tempfile::TempDir;

#[test]
fn profile_id_format() {
    assert_eq!(
        profile_id("openai-codex", "default"),
        "openai-codex:default"
    );
}

#[test]
fn token_expiry_math() {
    let token_set = TokenSet {
        access_token: "token".into(),
        refresh_token: Some("refresh".into()),
        id_token: None,
        expires_at: Some(Utc::now() + chrono::Duration::seconds(10)),
        token_type: Some("Bearer".into()),
        scope: None,
    };

    assert!(token_set.is_expiring_within(Duration::from_secs(15)));
    assert!(!token_set.is_expiring_within(Duration::from_secs(1)));
}

#[tokio::test]
async fn store_roundtrip_with_encryption() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);

    let mut profile = AuthProfile::new_oauth(
        "openai-codex",
        "default",
        TokenSet {
            access_token: "access-123".into(),
            refresh_token: Some("refresh-123".into()),
            id_token: None,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            token_type: Some("Bearer".into()),
            scope: Some("openid offline_access".into()),
        },
    );
    profile.account_id = Some("acct_123".into());

    store.upsert_profile(profile.clone(), true).unwrap();

    let data = store.load().unwrap();
    let loaded = data.profiles.get(&profile.id).unwrap();

    assert_eq!(loaded.provider, "openai-codex");
    assert_eq!(loaded.profile_name, "default");
    assert_eq!(loaded.account_id.as_deref(), Some("acct_123"));
    assert_eq!(
        loaded
            .token_set
            .as_ref()
            .and_then(|t| t.refresh_token.as_deref()),
        Some("refresh-123")
    );

    let raw = tokio::fs::read_to_string(store.path()).await.unwrap();
    assert!(raw.contains("enc2:"));
    assert!(!raw.contains("refresh-123"));
    assert!(!raw.contains("access-123"));
}

#[tokio::test]
async fn atomic_write_replaces_file() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let profile = AuthProfile::new_token("anthropic", "default", "token-abc".into());
    store.upsert_profile(profile, true).unwrap();

    let path = store.path().to_path_buf();
    assert!(path.exists());

    let contents = tokio::fs::read_to_string(path).await.unwrap();
    assert!(contents.contains("\"schema_version\": 1"));
}

#[test]
fn token_set_not_expiring_when_no_expiry() {
    let token_set = TokenSet {
        access_token: "token".into(),
        refresh_token: None,
        id_token: None,
        expires_at: None,
        token_type: None,
        scope: None,
    };
    assert!(!token_set.is_expiring_within(Duration::from_secs(3600)));
}

#[test]
fn auth_profile_new_token() {
    let profile = AuthProfile::new_token("anthropic", "default", "sk-abc".into());
    assert_eq!(profile.provider, "anthropic");
    assert_eq!(profile.profile_name, "default");
    assert_eq!(profile.kind, AuthProfileKind::Token);
    assert_eq!(profile.token.as_deref(), Some("sk-abc"));
    assert!(profile.token_set.is_none());
}

#[test]
fn auth_profile_new_oauth() {
    let ts = TokenSet {
        access_token: "access".into(),
        refresh_token: Some("refresh".into()),
        id_token: None,
        expires_at: None,
        token_type: None,
        scope: None,
    };
    let profile = AuthProfile::new_oauth("openai", "work", ts);
    assert_eq!(profile.kind, AuthProfileKind::OAuth);
    assert!(profile.token_set.is_some());
    assert!(profile.token.is_none());
}

#[test]
fn auth_profiles_data_default() {
    let data = AuthProfilesData::default();
    assert_eq!(data.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(data.profiles.is_empty());
    assert!(data.active_profiles.is_empty());
}

#[test]
fn remove_nonexistent_profile_returns_false() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let result = store.remove_profile("nonexistent:id").unwrap();
    assert!(!result);
}

#[test]
fn remove_existing_profile_returns_true() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("test", "default", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, true).unwrap();

    let removed = store.remove_profile(&id).unwrap();
    assert!(removed);

    let data = store.load().unwrap();
    assert!(!data.profiles.contains_key(&id));
    assert!(!data.active_profiles.values().any(|v| v == &id));
}

#[test]
fn set_active_profile_errors_for_missing_profile() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let err = store
        .set_active_profile("openai", "missing:id")
        .unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn set_active_profile_succeeds_for_existing_profile() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, false).unwrap();

    store.set_active_profile("openai", &id).unwrap();
    let data = store.load().unwrap();
    assert_eq!(data.active_profiles.get("openai"), Some(&id));
}

#[test]
fn clear_active_profile() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok".into());
    store.upsert_profile(profile, true).unwrap();

    store.clear_active_profile("openai").unwrap();
    let data = store.load().unwrap();
    assert!(data.active_profiles.get("openai").is_none());
}

#[test]
fn update_profile_modifies_in_place() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, false).unwrap();

    let updated = store
        .update_profile(&id, |p| {
            p.metadata.insert("env".into(), "staging".into());
            Ok(())
        })
        .unwrap();
    assert_eq!(
        updated.metadata.get("env").map(|s| s.as_str()),
        Some("staging")
    );
}

#[test]
fn update_profile_errors_for_missing_id() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let err = store.update_profile("missing:id", |_| Ok(())).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn upsert_preserves_created_at_on_update() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok1".into());
    let id = profile.id.clone();
    let created = profile.created_at;
    store.upsert_profile(profile, false).unwrap();

    std::thread::sleep(Duration::from_millis(10));
    let updated = AuthProfile::new_token("openai", "prod", "tok2".into());
    store.upsert_profile(updated, false).unwrap();

    let data = store.load().unwrap();
    let loaded = data.profiles.get(&id).unwrap();
    assert_eq!(loaded.created_at, created);
}

#[test]
fn auth_profile_kind_serde_roundtrip() {
    let json = serde_json::to_string(&AuthProfileKind::OAuth).unwrap();
    assert_eq!(json, "\"o-auth\""); // kebab-case
    let back: AuthProfileKind = serde_json::from_str(&json).unwrap();
    assert_eq!(back, AuthProfileKind::OAuth);

    let json = serde_json::to_string(&AuthProfileKind::Token).unwrap();
    assert_eq!(json, "\"token\"");
}
