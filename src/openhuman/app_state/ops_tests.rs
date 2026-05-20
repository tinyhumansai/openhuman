use super::*;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn sanitize_snapshot_user_drops_empty_payloads() {
    assert_eq!(sanitize_snapshot_user(Some(json!({}))), None);
    assert_eq!(sanitize_snapshot_user(Some(Value::Null)), None);
    assert_eq!(
        sanitize_snapshot_user(Some(json!({ "firstName": "steven" }))),
        Some(json!({ "firstName": "steven" }))
    );
}

fn make_cached_entry(age: Duration) -> CachedCurrentUser {
    CachedCurrentUser {
        api_base: "https://staging-api.tinyhumans.ai".to_string(),
        token: "tok".to_string(),
        fetched_at: Instant::now() - age,
        user: json!({ "firstName": "steven" }),
    }
}

// The freshness branch in `fetch_current_user_cached` is `elapsed() < TTL`.
// Lock that contract here so a future TTL change can't silently flip the
// cache from "hit" to "miss" without updating this test.
#[test]
fn cached_entry_is_considered_fresh_within_ttl() {
    let fresh = make_cached_entry(Duration::from_millis(0));
    assert!(fresh.fetched_at.elapsed() < CURRENT_USER_REFRESH_TTL);
}

#[test]
fn cached_entry_is_considered_expired_past_ttl() {
    let expired = make_cached_entry(CURRENT_USER_REFRESH_TTL + Duration::from_millis(50));
    assert!(expired.fetched_at.elapsed() >= CURRENT_USER_REFRESH_TTL);
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
fn resolve_base_normalizes_missing_trailing_slash() {
    let mut cfg = Config::default();
    cfg.api_url = Some("https://api.example.test/openhuman".into());

    let base = resolve_base(&cfg).expect("resolve_base");
    assert_eq!(base.as_str(), "https://api.example.test/");
}

#[test]
fn resolve_base_rejects_invalid_urls() {
    let mut cfg = Config::default();
    cfg.api_url = Some("://definitely-not-a-url".into());

    let err = resolve_base(&cfg).expect_err("invalid URL should fail");
    assert!(err.contains("invalid api_url"));
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
            local_model_consent_given: true,
            local_model_download_started: false,
            enabled_tools: vec!["search".into()],
            connected_sources: vec!["telegram".into()],
            updated_at_ms: Some(42),
        }),
    };

    save_stored_app_state(&cfg, &state).expect("save app state");
    let reloaded = load_stored_app_state(&cfg).expect("reload app state");
    assert_eq!(reloaded.encryption_key, Some("enc-key".into()));
    let tasks = reloaded.onboarding_tasks.expect("onboarding tasks");
    assert!(tasks.accessibility_permission_granted);
    assert!(tasks.local_model_consent_given);
    assert_eq!(tasks.enabled_tools, vec!["search".to_string()]);
    assert_eq!(tasks.connected_sources, vec!["telegram".to_string()]);
    assert_eq!(tasks.updated_at_ms, Some(42));
}

#[test]
fn peek_cached_current_user_identity_plucks_known_fields() {
    struct CacheResetGuard;
    impl Drop for CacheResetGuard {
        fn drop(&mut self) {
            *CURRENT_USER_CACHE.lock() = None;
        }
    }
    let _reset = CacheResetGuard;
    *CURRENT_USER_CACHE.lock() = Some(CachedCurrentUser {
        api_base: "https://api.example.test".into(),
        token: "tok".into(),
        fetched_at: Instant::now(),
        user: json!({
            "userId": "user-123",
            "display_name": "Alice Example",
            "email": "alice@example.test",
            "ignored": "x"
        }),
    });

    let identity = peek_cached_current_user_identity().expect("identity");
    assert_eq!(identity.id.as_deref(), Some("user-123"));
    assert_eq!(identity.name.as_deref(), Some("Alice Example"));
    assert_eq!(identity.email.as_deref(), Some("alice@example.test"));
}

#[test]
fn peek_cached_current_user_identity_returns_none_when_only_empty_fields_exist() {
    struct CacheResetGuard;
    impl Drop for CacheResetGuard {
        fn drop(&mut self) {
            *CURRENT_USER_CACHE.lock() = None;
        }
    }
    let _reset = CacheResetGuard;
    *CURRENT_USER_CACHE.lock() = Some(CachedCurrentUser {
        api_base: "https://api.example.test".into(),
        token: "tok".into(),
        fetched_at: Instant::now(),
        user: json!({
            "id": "   ",
            "name": "",
            "email": "   "
        }),
    });

    assert!(peek_cached_current_user_identity().is_none());
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
    let _reset = SnapshotCacheResetGuard;

    let dummy = build_dummy_runtime_snapshot();
    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: dummy.clone(),
        fetched_at: Instant::now(),
    });

    let cache = RUNTIME_SNAPSHOT_CACHE.lock();
    let entry = cache.as_ref().expect("cache should have entry");
    assert!(
        entry.fetched_at.elapsed() < RUNTIME_SNAPSHOT_TTL,
        "fresh entry should be within TTL"
    );
    assert_eq!(entry.snapshot.autocomplete.phase, dummy.autocomplete.phase);
}

#[test]
fn runtime_snapshot_cache_miss_after_ttl() {
    let _reset = SnapshotCacheResetGuard;

    *RUNTIME_SNAPSHOT_CACHE.lock() = Some(CachedRuntimeSnapshot {
        snapshot: build_dummy_runtime_snapshot(),
        fetched_at: Instant::now() - (RUNTIME_SNAPSHOT_TTL + Duration::from_millis(100)),
    });

    let cache = RUNTIME_SNAPSHOT_CACHE.lock();
    let entry = cache.as_ref().expect("cache should have entry");
    assert!(
        entry.fetched_at.elapsed() >= RUNTIME_SNAPSHOT_TTL,
        "stale entry should be past TTL"
    );
}

#[test]
fn degraded_runtime_snapshot_has_expected_degraded_fields() {
    let cfg = Config::default();
    let snapshot = degraded_runtime_snapshot(&cfg);

    assert_eq!(snapshot.autocomplete.phase, "degraded");
    assert_eq!(snapshot.local_ai.state, "disabled");
    assert!(
        matches!(
            snapshot.service.state,
            crate::openhuman::service::ServiceState::Unknown(_)
        ),
        "service state should be Unknown in degraded snapshot"
    );
    assert!(!snapshot.screen_intelligence.session.active);
}

#[test]
fn auth_fetch_timeout_constant_is_below_rpc_timeout() {
    // The 30s RPC timeout on the frontend means auth fetch + runtime snapshot
    // must fit comfortably. Verify the constants are sane.
    assert!(
        AUTH_FETCH_TIMEOUT.as_secs() < 15,
        "auth fetch timeout should be well under the 30s RPC timeout"
    );
    assert!(
        RUNTIME_SNAPSHOT_TIMEOUT.as_secs() < 20,
        "runtime snapshot timeout should be well under the 30s RPC timeout"
    );
    assert!(
        AUTH_FETCH_TIMEOUT + RUNTIME_SNAPSHOT_TIMEOUT < Duration::from_secs(30),
        "total of auth + runtime timeouts must fit within the 30s RPC timeout"
    );
}

fn build_dummy_runtime_snapshot() -> RuntimeSnapshot {
    degraded_runtime_snapshot(&Config::default())
}
