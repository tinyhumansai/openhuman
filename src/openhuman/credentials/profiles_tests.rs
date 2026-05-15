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
fn corrupt_store_is_quarantined_and_reset() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    let path = store.path().to_path_buf();

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, b"{ not valid json").unwrap();

    let data = store.load().unwrap();
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
    store.upsert_profile(profile, true).unwrap();
    let reloaded = store.load().unwrap();
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
#[test]
fn load_drops_profiles_whose_decryption_fails_under_rotated_key() {
    // The SecretStore caches keys by canonicalised path in a process-wide
    // OnceCell. Use a fresh temp dir per test so we don't pick up a
    // sibling test's cached key.
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), true);

    // Seed two profiles. One ("doomed") will be made unrecoverable by
    // rewriting the encrypted token under a new key; the other
    // ("plain-fine") uses kind=Token with a plaintext token that the
    // legacy `enc:` / plaintext branch decrypts trivially, so even
    // after key rotation it survives.
    let doomed = AuthProfile::new_token("app-session", "default", "real-jwt-payload".into());
    store.upsert_profile(doomed.clone(), true).unwrap();

    // Manually corrupt the persisted token: rewrite it as a syntactically
    // valid enc2: hex blob that the *current* key cannot decrypt.
    // (Easier than rotating the key file because the SecretStore caches
    // by canonical path.)
    let path = store.path().to_path_buf();
    let mut data: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let profile_id = doomed.id.clone();
    data["profiles"][&profile_id]["token"] = serde_json::Value::String(
        // 12-byte nonce + 32 bytes of "ciphertext" that won't authenticate
        // under any random key — hex-encoded, prefixed with enc2:.
        "enc2:000102030405060708090a0b\
              deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
            .to_string(),
    );
    std::fs::write(&path, serde_json::to_string_pretty(&data).unwrap()).unwrap();

    // First load: should silently drop the doomed profile rather than
    // bubbling the decrypt error and breaking every poll.
    let loaded = store.load().expect(
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
    let loaded2 = store.load().unwrap();
    assert!(!loaded2.profiles.contains_key(&profile_id));
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
fn auth_profile_lock_errors_do_not_include_local_paths() {
    let tmp = TempDir::new().unwrap();
    let invalid_state_dir = tmp.path().join("not-a-directory");
    std::fs::write(&invalid_state_dir, "occupied").unwrap();

    let store = AuthProfilesStore::new(&invalid_state_dir, false);
    let err = store.load().unwrap_err().to_string();

    assert!(err.contains("Failed to create auth profile lock directory"));
    assert!(!err.contains(&tmp.path().display().to_string()));
    assert!(!err.contains(&invalid_state_dir.display().to_string()));
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

// --- Issue #1612: stale auth-profiles.lock recovery -----------------------

/// A pid we expect to be safely above any real process id on macOS / Linux /
/// Windows test runners. Used to simulate a lock file written by a process
/// that has since exited.
const SYNTHETIC_DEAD_PID: u32 = i32::MAX as u32;

#[test]
fn is_pid_alive_detects_current_process() {
    assert!(is_pid_alive(std::process::id()));
}

#[test]
fn is_pid_alive_returns_false_for_synthetic_dead_pid() {
    assert!(!is_pid_alive(SYNTHETIC_DEAD_PID));
}

#[test]
fn acquire_lock_clears_stale_lock_with_dead_pid() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, format!("pid={SYNTHETIC_DEAD_PID}\n")).unwrap();
    assert!(lock_path.exists());

    // A no-op call that goes through acquire_lock should succeed quickly
    // by recognising the previous lock as stale and removing it.
    let data = store.load().unwrap();
    assert!(data.profiles.is_empty());
    assert!(
        !lock_path.exists(),
        "guard should have removed the lock on drop"
    );
}

#[test]
fn acquire_lock_recovers_after_upsert_when_dead_pid_lock_left_behind() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // Pre-existing lock from a crashed previous run.
    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, format!("pid={SYNTHETIC_DEAD_PID}\n")).unwrap();

    let profile = AuthProfile::new_token("openai", "default", "tok".into());
    let id = profile.id.clone();
    store.upsert_profile(profile, true).unwrap();

    let data = store.load().unwrap();
    assert!(data.profiles.contains_key(&id));
    assert!(!lock_path.exists());
}

#[test]
fn clear_lock_if_stale_leaves_live_pid_alone() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, format!("pid={}\n", std::process::id())).unwrap();

    assert!(!store.clear_lock_if_stale());
    assert!(lock_path.exists(), "lock for live pid must not be removed");
}

#[test]
fn clear_lock_if_stale_leaves_malformed_lock_alone() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    let lock_path = tmp.path().join(LOCK_FILENAME);
    std::fs::write(&lock_path, "garbage without a pid line\n").unwrap();

    assert!(!store.clear_lock_if_stale());
    assert!(
        lock_path.exists(),
        "malformed lock should not be auto-removed; fall back to busy-wait + timeout"
    );
}

#[test]
fn clear_lock_if_stale_is_noop_when_lock_missing() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);
    assert!(!store.clear_lock_if_stale());
}

#[test]
fn acquire_lock_writes_pid_so_future_callers_can_recover() {
    let tmp = TempDir::new().unwrap();
    let store = AuthProfilesStore::new(tmp.path(), false);

    // Drive a real acquire/release cycle and snapshot the on-disk lock
    // while the guard is held.
    let lock_path = tmp.path().join(LOCK_FILENAME);
    let observed = {
        let _guard = store.acquire_lock().unwrap();
        std::fs::read_to_string(&lock_path).unwrap()
    };
    assert!(
        observed.contains(&format!("pid={}", std::process::id())),
        "lock file should embed the owning pid, got {observed:?}"
    );
    assert!(!lock_path.exists(), "guard must remove lock on drop");
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
