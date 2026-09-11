use super::*;
use once_cell::sync::Lazy as TestLazy;
use parking_lot::Mutex as TestMutex;
use serde_json::json;
use tempfile::tempdir;

/// Serialises every test that reads or writes the process-global snapshot
/// caches. A `tokio` mutex rather than a `parking_lot` one so async tests can
/// hold it across an `.await` — sibling `ops_current_user_backoff_tests.rs`
/// guards its own global the same way.
///
/// `pub(super)` for `ops_snapshot_latency_tests.rs`, which seeds the positive
/// cache and so has to serialise against the readers here.
pub(super) static APP_STATE_CACHE_TEST_LOCK: TestLazy<tokio::sync::Mutex<()>> =
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
        keyring_consent: None,
    };

    save_app_state(&cfg, &state).expect("save app state");
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.blocking_lock();
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
            crate::openhuman::platform::service::ServiceState::Unknown(_)
        ),
        "service state should be Unknown in degraded snapshot"
    );
}

#[test]
fn auth_fetch_timeout_constant_is_below_rpc_timeout() {
    // The 30s RPC timeout on the frontend means auth fetch + runtime snapshot
    // must fit comfortably. Asserted across the whole configurable range
    // (#5930), not just the default, because the operator override is what
    // could push the pair past the ceiling.
    assert!(
        MAX_AUTH_FETCH_TIMEOUT_SECS < 15,
        "auth fetch timeout should be well under the 30s RPC timeout"
    );
    assert!(
        RUNTIME_SNAPSHOT_TIMEOUT.as_secs() < 20,
        "runtime snapshot timeout should be well under the 30s RPC timeout"
    );
    assert!(
        Duration::from_secs(MAX_AUTH_FETCH_TIMEOUT_SECS) + RUNTIME_SNAPSHOT_TIMEOUT
            < Duration::from_secs(30),
        "even the widest permitted auth timeout plus the runtime timeout must fit within the 30s RPC timeout"
    );
    assert!(
        (MIN_AUTH_FETCH_TIMEOUT_SECS..=MAX_AUTH_FETCH_TIMEOUT_SECS)
            .contains(&DEFAULT_AUTH_FETCH_TIMEOUT_SECS),
        "the default must itself be an accepted override value"
    );
}

// ── Configurable auth fetch timeout (#5930) ─────────────────────────────────
//
// "5s may be too tight" is the issue's first acceptance criterion. The risk in
// answering it is that a wider timeout re-opens #5624: the backoff's first step
// was a fixed 10s, so an operator setting 20s would make every poll find the
// window already closed and pay the full 20s again. The clamp and the derived
// base are what stop that, so both are pinned here.

#[test]
fn parse_auth_fetch_timeout_secs_clamps_and_falls_back() {
    assert_eq!(
        parse_auth_fetch_timeout_secs(None),
        DEFAULT_AUTH_FETCH_TIMEOUT_SECS,
        "an unset override leaves the default in place"
    );
    assert_eq!(
        parse_auth_fetch_timeout_secs(Some("not-a-number")),
        DEFAULT_AUTH_FETCH_TIMEOUT_SECS
    );
    assert_eq!(
        parse_auth_fetch_timeout_secs(Some("")),
        DEFAULT_AUTH_FETCH_TIMEOUT_SECS
    );
    assert_eq!(
        parse_auth_fetch_timeout_secs(Some("0")),
        DEFAULT_AUTH_FETCH_TIMEOUT_SECS,
        "0 would disable the timeout entirely and is rejected"
    );
    assert_eq!(
        parse_auth_fetch_timeout_secs(Some(&(MIN_AUTH_FETCH_TIMEOUT_SECS - 1).to_string())),
        DEFAULT_AUTH_FETCH_TIMEOUT_SECS,
        "below the floor is rejected rather than clamped up, so a typo is visible in the logs"
    );
    assert_eq!(
        parse_auth_fetch_timeout_secs(Some(&(MAX_AUTH_FETCH_TIMEOUT_SECS + 1).to_string())),
        DEFAULT_AUTH_FETCH_TIMEOUT_SECS,
        "above the ceiling is rejected rather than clamped down"
    );
    assert_eq!(
        parse_auth_fetch_timeout_secs(Some("  7  ")),
        7,
        "surrounding whitespace is tolerated"
    );

    for secs in MIN_AUTH_FETCH_TIMEOUT_SECS..=MAX_AUTH_FETCH_TIMEOUT_SECS {
        assert_eq!(
            parse_auth_fetch_timeout_secs(Some(&secs.to_string())),
            secs,
            "{secs}s is inside the accepted range and must pass through unchanged"
        );
    }
}

#[test]
fn the_derived_backoff_base_outlasts_every_permitted_fetch_timeout() {
    // This is #5624's invariant, restated as a property over the range #5930
    // opened up. A first step shorter than the fetch timeout means the next
    // poll finds the window already closed and pays the full timeout again —
    // which is the bug, not the fix.
    for secs in MIN_AUTH_FETCH_TIMEOUT_SECS..=MAX_AUTH_FETCH_TIMEOUT_SECS {
        let timeout = Duration::from_secs(secs);
        let base = current_user_backoff_base_for(timeout);
        assert!(
            base > timeout,
            "first backoff step {base:?} must exceed the fetch timeout {timeout:?}"
        );
        assert!(
            base > CURRENT_USER_REFRESH_TTL,
            "first backoff step {base:?} must exceed the cache TTL {CURRENT_USER_REFRESH_TTL:?}"
        );
        assert!(
            base <= CURRENT_USER_BACKOFF_MAX,
            "first backoff step {base:?} must not start at or above the cap {CURRENT_USER_BACKOFF_MAX:?}"
        );
    }
}

fn build_dummy_runtime_snapshot() -> RuntimeSnapshot {
    degraded_runtime_snapshot(&Config::default())
}

/// `fetch_current_user` hand-rolls its own TLS client rather than borrowing
/// `BackendOAuthClient`'s, so it inherits nothing from that path's default
/// headers — this call went out unattributed until review caught it. Assert on
/// the wire rather than on `build_client`'s configuration: `reqwest::Client`
/// exposes no way to read its default headers back, so the only proof the
/// header survives into a real request is a real request.
#[tokio::test]
async fn current_user_fetch_carries_the_product_identity() {
    use axum::extract::State;
    use axum::http::HeaderMap;
    use axum::routing::get;
    use axum::{Json, Router};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::net::TcpListener;

    // The identity is process-global, so serialise against every other test
    // that reads or writes it — otherwise this races an override installed by
    // `api::product`'s or `api::rest`'s tests and flakes on the default.
    let _identity_guard = crate::api::product::product_identity_test_lock();
    crate::api::product::reset_product_identity_for_test();

    type Captured = Arc<StdMutex<Option<String>>>;

    async fn auth_me(State(captured): State<Captured>, headers: HeaderMap) -> Json<Value> {
        *captured.lock().unwrap() = headers
            .get(crate::api::product::PRODUCT_IDENTITY_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_string);
        Json(json!({ "success": true, "data": { "firstName": "steven" } }))
    }

    let captured: Captured = Arc::new(StdMutex::new(None));
    let app = Router::new()
        .route("/auth/me", get(auth_me))
        .with_state(captured.clone());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let config = Config {
        api_url: Some(format!("http://{addr}")),
        ..Config::default()
    };
    fetch_current_user(&config, "tok")
        .await
        .expect("the stub answers /auth/me");

    assert_eq!(
        captured.lock().unwrap().as_deref(),
        Some(crate::api::product::DEFAULT_PRODUCT_IDENTITY),
        "GET /auth/me must carry the product identity header"
    );

    crate::api::product::reset_product_identity_for_test();
}

// Serialises the `OPENHUMAN_WORKSPACE` env mutations below so two of these tests
// can't race each other on the process-global var.
static WORKSPACE_ENV_TEST_LOCK: TestLazy<TestMutex<()>> = TestLazy::new(|| TestMutex::new(()));

/// RAII guard for `OPENHUMAN_WORKSPACE`. Captures the prior value on
/// construction and restores it (set or remove) on drop, so a test that panics
/// between the mutation and the end of the test can't leak the override into a
/// sibling test. Must be constructed while holding `WORKSPACE_ENV_TEST_LOCK`:
/// mutating a process env var while another thread reads it is unsafe, and the
/// lock serialises every test in this group.
struct WorkspaceEnvGuard {
    prior: Option<std::ffi::OsString>,
}

impl WorkspaceEnvGuard {
    fn set(value: &std::path::Path) -> Self {
        let prior = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", value);
        Self { prior }
    }

    fn set_empty() -> Self {
        let prior = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::set_var("OPENHUMAN_WORKSPACE", "");
        Self { prior }
    }

    fn unset() -> Self {
        let prior = std::env::var_os("OPENHUMAN_WORKSPACE");
        std::env::remove_var("OPENHUMAN_WORKSPACE");
        Self { prior }
    }
}

impl Drop for WorkspaceEnvGuard {
    fn drop(&mut self) {
        match &self.prior {
            Some(value) => std::env::set_var("OPENHUMAN_WORKSPACE", value),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }
}

/// The #6079 twin: `config_dir_for_workspace_env` must resolve the modern
/// `<root>/.openhuman/workspace` layout to its parent `<root>/.openhuman` (the
/// real config dir), NOT the doubled `<root>/.openhuman/.openhuman`. The private
/// reimplementation this replaced produced the doubled path, so
/// `config_is_workspace_env_scoped` disagreed with the loader and mis-scoped
/// credentials on session revalidation. Delegating to the shared
/// `resolve_config_dir_for_workspace` keeps the two in lockstep.
///
/// The workspace root is named `default_root_dir_name()` (`.openhuman` /
/// `.openhuman-staging`) so the modern-layout arm — which keys on that name —
/// fires regardless of the ambient `OPENHUMAN_APP_ENV`, and a temp dir isolates
/// it from any real `.openhuman` on the host.
#[test]
fn config_dir_for_workspace_env_modern_layout_does_not_double_openhuman() {
    let _g = WORKSPACE_ENV_TEST_LOCK.lock();
    let tmp = tempdir().unwrap();
    let root = tmp
        .path()
        .join(crate::openhuman::config::default_root_dir_name());
    let workspace = root.join("workspace");
    let _env = WorkspaceEnvGuard::set(&workspace);

    let resolved = config_dir_for_workspace_env();

    assert_eq!(resolved, Some(root.clone()));
    assert_ne!(
        resolved,
        Some(root.join(crate::openhuman::config::default_root_dir_name())),
        "must never return the doubled .openhuman/.openhuman path"
    );
}

/// A fresh legacy layout (`<proj>/workspace` with no sibling `.openhuman` on
/// disk) must resolve to the sibling `<proj>/.openhuman`, matching the loader —
/// not nest the workspace inside itself. Guards the same seam as the
/// `dirs.rs` fresh-legacy test, one level up through the app_state resolver.
#[test]
fn config_dir_for_workspace_env_fresh_legacy_resolves_to_sibling() {
    let _g = WORKSPACE_ENV_TEST_LOCK.lock();
    let tmp = tempdir().unwrap();
    let project = tmp.path().join("some-project");
    let workspace = project.join("workspace");
    let _env = WorkspaceEnvGuard::set(&workspace);

    let resolved = config_dir_for_workspace_env();

    assert_eq!(
        resolved,
        Some(project.join(".openhuman")),
        "a fresh legacy workspace must resolve to its sibling .openhuman"
    );
}

/// An unset / empty `OPENHUMAN_WORKSPACE` yields `None` so the caller falls back
/// to user-scoped resolution rather than treating the empty string as a path.
#[test]
fn config_dir_for_workspace_env_none_when_unset_or_empty() {
    let _g = WORKSPACE_ENV_TEST_LOCK.lock();
    {
        let _env = WorkspaceEnvGuard::unset();
        assert_eq!(config_dir_for_workspace_env(), None);
    }

    let _env = WorkspaceEnvGuard::set_empty();
    assert_eq!(
        config_dir_for_workspace_env(),
        None,
        "an empty override must not be treated as a path"
    );
}
