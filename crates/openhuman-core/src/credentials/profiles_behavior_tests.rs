use super::*;
use tempfile::TempDir;

#[tokio::test]
async fn profile_id_format() {
    assert_eq!(
        profile_id("openai-codex", "default"),
        "openai-codex:default"
    );
}

#[tokio::test]
async fn token_expiry_math() {
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

    store.upsert_profile(profile.clone(), true).await.unwrap();

    let data = store.load().await.unwrap();
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

    // Under the keychain-backed model (FileBackend in debug builds, real OS
    // keychain in release), secret fields are stored in the keychain and
    // omitted from the JSON file entirely. The on-disk JSON must not leak
    // the plaintext secrets in any form.
    let raw = tokio::fs::read_to_string(store.path()).await.unwrap();
    assert!(!raw.contains("refresh-123"));
    assert!(!raw.contains("access-123"));
}

#[tokio::test]
async fn profile_write_publishes_an_encrypted_document() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let profile = AuthProfile::new_token("anthropic", "default", "token-abc".into());
    store.upsert_profile(profile, true).await.unwrap();

    let path = store.path().to_path_buf();
    assert!(path.exists());

    let bytes = tokio::fs::read(path).await.unwrap();
    assert!(!is_persisted_document(&bytes));
    assert!(!String::from_utf8_lossy(&bytes).contains("token-abc"));
}

#[tokio::test]
async fn token_set_not_expiring_when_no_expiry() {
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

#[tokio::test]
async fn auth_profile_new_token() {
    let profile = AuthProfile::new_token("anthropic", "default", "sk-abc".into());
    assert_eq!(profile.provider, "anthropic");
    assert_eq!(profile.profile_name, "default");
    assert_eq!(profile.kind, AuthProfileKind::Token);
    assert_eq!(profile.token.as_deref(), Some("sk-abc"));
    assert!(profile.token_set.is_none());
}

#[tokio::test]
async fn auth_profile_new_oauth() {
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

#[tokio::test]
async fn auth_profiles_data_default() {
    let data = AuthProfilesData::default();
    assert_eq!(data.schema_version, CURRENT_SCHEMA_VERSION);
    assert!(data.profiles.is_empty());
    assert!(data.active_profiles.is_empty());
}

#[tokio::test]
async fn corrupt_store_is_quarantined_and_reset() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let path = store.legacy_path.clone();

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ not valid json").unwrap();

    let data = store.load().await.unwrap();
    assert!(data.profiles.is_empty());
    assert_eq!(data.schema_version, CURRENT_SCHEMA_VERSION);

    let parent = path.parent().unwrap();
    let quarantined: Vec<_> = std::fs::read_dir(parent)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".corrupt-"))
        .collect();
    assert_eq!(quarantined.len(), 1, "expected one quarantined file");

    let profile = AuthProfile::new_token("openai", "default", "tok".into());
    store.upsert_profile(profile, true).await.unwrap();
    let reloaded = store.load().await.unwrap();
    assert_eq!(reloaded.profiles.len(), 1);
}

/// When the encrypted-secrets key file has rotated between writes and reads
/// (e.g. `.secret_key` got regenerated underneath an existing
/// auth-profiles.json — observed when a workspace gets partially restored
/// or when OPENHUMAN_WORKSPACE points at a half-populated test dir), the
/// store must silently drop the unrecoverable profile and rewrite the
/// file. Without this, `app_state_snapshot` polls infinite-loop on
/// "Decryption failed — wrong key or tampered data" and the user can
/// never log in cleanly because every read pre-empts before reaching
/// the "no profile" code path.

#[tokio::test]
async fn load_drops_profiles_whose_decryption_fails_under_rotated_key() {
    // The SecretStore caches keys by canonicalised path in a process-wide
    // OnceCell. Use a fresh temp dir per test so we don't pick up a
    // sibling test's cached key.
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);

    // Seed one profile, then replace its token with malformed enc2 data and
    // add a valid plaintext legacy profile to verify selective cleanup.
    let doomed = AuthProfile::new_token("app-session", "default", "real-jwt-payload".into());
    store.upsert_profile(doomed.clone(), true).await.unwrap();

    // Profile tokens are inside the TSD-encrypted profile document. Plant a
    // malformed field-level enc2 value to verify drop-on-decrypt-failure.
    let profile_id = doomed.id.clone();
    let mut data = store.read_persisted_locked().unwrap();
    data.profiles.get_mut(&profile_id).unwrap().token = Some(
        "enc2:000102030405060708090a0bdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
    );
    data.profiles.insert(
        "legacy:plain-fine".into(),
        serde_json::from_value(serde_json::json!({
            "provider": "legacy",
            "profile_name": "plain-fine",
            "kind": "token",
            "token": "legacy-plain-token",
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z"
        }))
        .unwrap(),
    );
    store.write_persisted_locked(&data).unwrap();

    // First load: should silently drop the doomed profile rather than
    // bubbling the decrypt error and breaking every poll.
    let loaded = store.load().await.expect(
        "load must succeed by dropping unrecoverable profiles, not by propagating decrypt errors",
    );
    assert!(
        !loaded.profiles.contains_key(&profile_id),
        "doomed profile must be purged from the in-memory view"
    );
    assert!(
        !loaded.active_profiles.values().any(|v| v == &profile_id),
        "active_profiles pointer to the doomed profile must also be cleared"
    );

    // Subsequent load: file was rewritten without the bad profile, so
    // there's nothing to drop on the second pass — same clean state.
    let loaded2 = store.load().await.unwrap();
    assert!(!loaded2.profiles.contains_key(&profile_id));
}

/// A persisted profile whose `kind` string is something the current code
/// doesn't recognise (e.g. legacy "OAuth" written before the kebab-case
/// rename, or "api_key" written by an older code path) must not poison
/// the whole load — otherwise *every* profile becomes unreadable and the
/// user is locked out of all sessions. Drop just the bad entry, matching
/// the decrypt-failure recovery pattern.

#[tokio::test]
async fn load_drops_profiles_with_unrecognized_kind_instead_of_failing_load() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // Seed one valid profile so we can verify the rest of the store survives.
    let good = AuthProfile::new_token("openai", "good", "tok-good".into());
    let good_id = good.id.clone();
    store.upsert_profile(good, true).await.unwrap();

    // Inject two profiles with kinds the current parser rejects:
    //   - "api_key": observed in Sentry issue #123 (370 events over 14d)
    //   - "OAuth"  : observed in Sentry issue #2605 (258 events) — the
    //                pre-kebab-case serialized form
    let mut data = store.read_persisted_locked().unwrap();
    data.profiles.insert(
        "legacy:apikey".into(),
        serde_json::from_value(serde_json::json!({
            "provider": "legacy",
            "profile_name": "apikey",
            "kind": "api_key",
            "token": "raw-token",
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z",
        }))
        .unwrap(),
    );
    data.profiles.insert(
        "legacy:oauth".into(),
        serde_json::from_value(serde_json::json!({
            "provider": "legacy",
            "profile_name": "oauth",
            "kind": "OAuth",
            "access_token": "raw-access",
            "created_at": "2025-01-01T00:00:00Z",
            "updated_at": "2025-01-01T00:00:00Z",
        }))
        .unwrap(),
    );
    data.active_profiles
        .insert("legacy".into(), "legacy:apikey".into());
    store.write_persisted_locked(&data).unwrap();

    // The load must succeed — the only failure mode prior to the fix was
    // bailing the entire load on the first unrecognized kind.
    let loaded = store
        .load()
        .await
        .expect("load must succeed by dropping profiles with unrecognized kinds");

    assert!(
        loaded.profiles.contains_key(&good_id),
        "the valid profile must survive"
    );
    assert!(
        !loaded.profiles.contains_key("legacy:apikey"),
        "profile with kind=api_key must be dropped"
    );
    assert!(
        !loaded.profiles.contains_key("legacy:oauth"),
        "profile with kind=OAuth (legacy casing) must be dropped"
    );
    assert!(
        !loaded
            .active_profiles
            .values()
            .any(|v| v == "legacy:apikey"),
        "active_profiles pointer to a dropped profile must be cleared"
    );

    // Subsequent load reads the cleaned encrypted document.
    let reread = store.read_persisted_locked().unwrap();
    assert!(!reread.profiles.contains_key("legacy:apikey"));
    assert!(!reread.profiles.contains_key("legacy:oauth"));
    assert!(reread.profiles.contains_key(&good_id));
}

#[tokio::test]
async fn remove_nonexistent_profile_returns_false() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let result = store.remove_profile("nonexistent:id").await.unwrap();
    assert!(!result);
}

#[tokio::test]
async fn remove_existing_profile_returns_true() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("test", "default", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, true).await.unwrap();

    let removed = store.remove_profile(&id).await.unwrap();
    assert!(removed);

    let data = store.load().await.unwrap();
    assert!(!data.profiles.contains_key(&id));
    assert!(!data.active_profiles.values().any(|v| v == &id));
}

#[tokio::test]
async fn set_active_profile_errors_for_missing_profile() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let err = store
        .set_active_profile("openai", "missing:id")
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn set_active_profile_succeeds_for_existing_profile() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, false).await.unwrap();

    store.set_active_profile("openai", &id).await.unwrap();
    let data = store.load().await.unwrap();
    assert_eq!(data.active_profiles.get("openai"), Some(&id));
}

#[tokio::test]
async fn clear_active_profile() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok".into());
    store.upsert_profile(profile, true).await.unwrap();

    store.clear_active_profile("openai").await.unwrap();
    let data = store.load().await.unwrap();
    assert!(!data.active_profiles.contains_key("openai"));
}

#[tokio::test]
async fn auth_profile_lock_errors_do_not_include_local_paths() {
    let tmp = TempDir::new().unwrap();
    let invalid_state_dir = tmp.path().join("not-a-directory");
    std::fs::write(&invalid_state_dir, "occupied").unwrap();

    let store = AuthProfilesStore::new(&invalid_state_dir, false);
    let err = store.load().await.unwrap_err().to_string();

    assert!(err.contains("Failed to acquire auth profile lease"));
    assert!(!err.contains(&tmp.path().display().to_string()));
    assert!(!err.contains(&invalid_state_dir.display().to_string()));
}

#[tokio::test]
async fn update_profile_modifies_in_place() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, false).await.unwrap();

    let updated = store
        .update_profile(&id, |p| {
            p.metadata.insert("env".into(), "staging".into());
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        updated.metadata.get("env").map(|s| s.as_str()),
        Some("staging")
    );
}

#[tokio::test]
async fn update_profile_errors_for_missing_id() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let err = store
        .update_profile("missing:id", |_| Ok(()))
        .await
        .unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[tokio::test]
async fn upsert_preserves_created_at_on_update() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let profile = AuthProfile::new_token("openai", "prod", "tok1".into());
    let id = profile.id.clone();
    let created = profile.created_at;
    store.upsert_profile(profile, false).await.unwrap();

    std::thread::sleep(Duration::from_millis(10));
    let updated = AuthProfile::new_token("openai", "prod", "tok2".into());
    store.upsert_profile(updated, false).await.unwrap();

    let data = store.load().await.unwrap();
    let loaded = data.profiles.get(&id).unwrap();
    assert_eq!(loaded.created_at, created);
}
