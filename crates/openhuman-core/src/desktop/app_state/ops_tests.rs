use super::*;
use once_cell::sync::Lazy as TestLazy;
use serde_json::json;
use tempfile::tempdir;

/// Serialises every test that reads or writes the process-global runtime
/// snapshot cache. A `tokio` mutex rather than a `parking_lot` one so async
/// tests can hold it across an `.await`.
static APP_STATE_CACHE_TEST_LOCK: TestLazy<tokio::sync::Mutex<()>> =
    TestLazy::new(|| tokio::sync::Mutex::new(()));

#[test]
fn sanitize_snapshot_user_drops_empty_payloads() {
    assert_eq!(sanitize_snapshot_user(Some(json!({}))), None);
    assert_eq!(sanitize_snapshot_user(Some(Value::Null)), None);
    assert_eq!(
        sanitize_snapshot_user(Some(json!({ "firstName": "steven" }))),
        Some(json!({ "firstName": "steven" }))
    );
}

#[test]
fn app_state_path_creates_state_dir_and_points_at_app_state_json() {
    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");

    let path = app_state_path(&cfg).expect("app_state_path");
    assert!(path.ends_with("state/app-state.json"));
    assert!(
        cfg.workspace_dir.join("state").is_dir(),
        "state dir should be created eagerly"
    );
}

#[test]
fn load_stored_app_state_returns_default_when_missing() {
    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");

    let state = load_stored_app_state(&cfg).expect("load default app state");
    assert!(state.encryption_key.is_none());
    assert!(state.onboarding_tasks.is_none());
}

#[test]
fn load_stored_app_state_quarantines_invalid_json_and_returns_default() {
    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");

    let path = app_state_path(&cfg).expect("app_state_path");
    std::fs::write(&path, "{ definitely not valid json").unwrap();

    let state = load_stored_app_state(&cfg).expect("load invalid app state");
    assert!(state.encryption_key.is_none());
    assert!(state.onboarding_tasks.is_none());
    assert!(
        !path.exists(),
        "invalid source file should be quarantined or removed"
    );

    let state_dir = path.parent().expect("state dir");
    let quarantined: Vec<_> = std::fs::read_dir(state_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("app-state.json.corrupted."))
        .collect();
    assert_eq!(quarantined.len(), 1, "expected one quarantined copy");
}

#[test]
fn save_and_reload_stored_app_state_round_trips() {
    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");

    let state = StoredAppState {
        encryption_key: Some("enc-key".into()),
        onboarding_tasks: Some(StoredOnboardingTasks {
            accessibility_permission_granted: true,
            enabled_tools: vec!["search".into()],
            connected_sources: vec!["telegram".into()],
            updated_at_ms: Some(42),
        }),
        keyring_consent: None,
    };

    save_app_state(&cfg, &state).expect("save app state");
    let reloaded = load_stored_app_state(&cfg).expect("reload app state");
    assert_eq!(reloaded.encryption_key, Some("enc-key".into()));
    let tasks = reloaded.onboarding_tasks.expect("onboarding tasks");
    assert!(tasks.accessibility_permission_granted);
    assert_eq!(tasks.enabled_tools, vec!["search".to_string()]);
    assert_eq!(tasks.connected_sources, vec!["telegram".to_string()]);
    assert_eq!(tasks.updated_at_ms, Some(42));
}

/// App-state files written before local model downloads were removed carry
/// `localModelConsentGiven` / `localModelDownloadStarted`. They must still
/// load (the rest of the onboarding state intact) and the stale keys must be
/// dropped on the next save.
#[test]
fn legacy_app_state_with_local_model_download_flags_still_loads() {
    let legacy = serde_json::json!({
        "encryptionKey": "enc-key",
        "onboardingTasks": {
            "accessibilityPermissionGranted": true,
            "localModelConsentGiven": true,
            "localModelDownloadStarted": true,
            "enabledTools": ["search"],
            "connectedSources": [],
            "updatedAtMs": 7
        }
    });
    let state: StoredAppState =
        serde_json::from_value(legacy).expect("legacy app state must deserialize");
    let tasks = state.onboarding_tasks.clone().expect("onboarding tasks");
    assert!(tasks.accessibility_permission_granted);
    assert_eq!(tasks.enabled_tools, vec!["search".to_string()]);
    assert_eq!(tasks.updated_at_ms, Some(7));

    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    save_app_state(&cfg, &state).expect("save app state");
    let reloaded = load_stored_app_state(&cfg).expect("reload app state");
    let serialized = serde_json::to_string(&reloaded).expect("serialize");
    assert!(!serialized.contains("localModelConsentGiven"));
    assert!(!serialized.contains("localModelDownloadStarted"));
}

// ── RuntimeSnapshot cache tests ──────────────────────────────────────────────

struct SnapshotCacheResetGuard;
impl Drop for SnapshotCacheResetGuard {
    fn drop(&mut self) {
        *RUNTIME_SNAPSHOT_CACHE.lock() = None;
    }
}

#[test]
fn runtime_snapshot_cache_hit_within_ttl() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
    let _reset = SnapshotCacheResetGuard;

    let dummy = build_dummy_runtime_snapshot();
    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: dummy.clone(),
        fetched_at: Instant::now(),
        config_key: std::path::PathBuf::new(),
    });

    let cache = RUNTIME_SNAPSHOT_CACHE.lock();
    let entry = cache.as_ref().expect("cache should have entry");
    assert!(
        entry.fetched_at.elapsed() < RUNTIME_SNAPSHOT_TTL,
        "fresh entry should be within TTL"
    );
    assert_eq!(entry.snapshot.local_ai.state, dummy.local_ai.state);
}

#[test]
fn runtime_snapshot_cache_miss_after_ttl() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
    let _reset = SnapshotCacheResetGuard;

    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: build_dummy_runtime_snapshot(),
        fetched_at: Instant::now() - (RUNTIME_SNAPSHOT_TTL + Duration::from_millis(100)),
        config_key: std::path::PathBuf::new(),
    });

    let cache = RUNTIME_SNAPSHOT_CACHE.lock();
    let entry = cache.as_ref().expect("cache should have entry");
    assert!(
        entry.fetched_at.elapsed() >= RUNTIME_SNAPSHOT_TTL,
        "stale entry should be past TTL"
    );
}

#[test]
fn fresh_cached_runtime_snapshot_returns_entry_within_ttl() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
    let _reset = SnapshotCacheResetGuard;

    let dummy = build_dummy_runtime_snapshot();
    let cfg = Config::default();
    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: dummy.clone(),
        fetched_at: Instant::now(),
        config_key: cfg.workspace_dir.clone(),
    });

    let served = fresh_cached_runtime_snapshot(&cfg, 1).expect("fresh entry should be served");
    assert_eq!(served.local_ai.state, dummy.local_ai.state);
}

#[test]
fn fresh_cached_runtime_snapshot_misses_when_stale_or_empty() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
    let _reset = SnapshotCacheResetGuard;

    let cfg = Config::default();

    // Empty cache → miss (forces the single-flight rebuild path).
    *RUNTIME_SNAPSHOT_CACHE.lock() = None;
    assert!(fresh_cached_runtime_snapshot(&cfg, 2).is_none());

    // Stale cache → miss, so the TTL bump can't silently keep serving old data.
    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: build_dummy_runtime_snapshot(),
        fetched_at: Instant::now() - (RUNTIME_SNAPSHOT_TTL + Duration::from_millis(100)),
        config_key: cfg.workspace_dir.clone(),
    });
    assert!(fresh_cached_runtime_snapshot(&cfg, 3).is_none());
}

#[test]
fn fresh_cached_runtime_snapshot_misses_on_config_key_mismatch() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
    let _reset = SnapshotCacheResetGuard;

    // A fresh entry cached for one workspace must never be served to another
    // config — a second user, or an E2E harness with an injected service mock,
    // has to rebuild against its own runtime instead of reading a foreign one.
    let mut owner = Config::default();
    owner.workspace_dir = std::path::PathBuf::from("/tmp/ws-owner");
    let mut other = Config::default();
    other.workspace_dir = std::path::PathBuf::from("/tmp/ws-other");

    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: build_dummy_runtime_snapshot(),
        fetched_at: Instant::now(),
        config_key: owner.workspace_dir.clone(),
    });

    assert!(
        fresh_cached_runtime_snapshot(&owner, 4).is_some(),
        "a config reads back its own fresh snapshot"
    );
    assert!(
        fresh_cached_runtime_snapshot(&other, 5).is_none(),
        "a foreign config misses instead of serving the wrong runtime"
    );
}

#[test]
fn degraded_runtime_snapshot_has_expected_degraded_fields() {
    let cfg = Config::default();
    let snapshot = degraded_runtime_snapshot(&cfg);

    assert_eq!(snapshot.local_ai.state, "disabled");
    assert!(
        matches!(
            snapshot.service.state,
            openhuman_platform::service::ServiceState::Unknown(_)
        ),
        "service state should be Unknown in degraded snapshot"
    );
}

// ── Configurable auth fetch timeout (#5930) ─────────────────────────────────
//
// "5s may be too tight" is the issue's first acceptance criterion. The risk in
// answering it is that a wider timeout re-opens #5624: the backoff's first step
// was a fixed 10s, so an operator setting 20s would make every poll find the
// window already closed and pay the full 20s again. The clamp and the derived
// base are what stop that, so both are pinned here.

fn build_dummy_runtime_snapshot() -> RuntimeSnapshot {
    degraded_runtime_snapshot(&Config::default())
}
