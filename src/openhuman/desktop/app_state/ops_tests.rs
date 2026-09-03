use super::*;
use once_cell::sync::Lazy as TestLazy;
use parking_lot::Mutex as TestMutex;
use serde_json::json;
use tempfile::tempdir;

static APP_STATE_CACHE_TEST_LOCK: TestLazy<TestMutex<()>> = TestLazy::new(|| TestMutex::new(()));

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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
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

// ── Current-user failure backoff (#5624) ────────────────────────────────────
//
// While the backend is unreachable, every `app_state_snapshot` poll used to
// re-attempt `auth_get_me` and re-pay the full `AUTH_FETCH_TIMEOUT`, because a
// failure was never recorded anywhere: `fetch_current_user_cached` cached only
// successes, and on a timeout its future was dropped before it could cache
// anything at all. 51 timeouts in one session is what that costs at a 5s poll
// cadence. These cover the record, the window, and the fact that the fetch
// actually consults it.

/// Serializes the tests that seed `CURRENT_USER_FAILURE`.
///
/// Async-aware rather than the `parking_lot` guard the positive-cache tests
/// use, because one of these tests has to hold it across an `.await` — the
/// whole point of that test is that `fetch_current_user_cached` consults the
/// record. Kept distinct from `APP_STATE_CACHE_TEST_LOCK` because the two guard
/// different globals and nothing here writes the positive cache.
static CURRENT_USER_FAILURE_TEST_LOCK: TestLazy<tokio::sync::Mutex<()>> =
    TestLazy::new(|| tokio::sync::Mutex::new(()));

/// Drops the seeded outage on the way out, so one test cannot leak into the next.
struct CurrentUserFailureResetGuard;

impl Drop for CurrentUserFailureResetGuard {
    fn drop(&mut self) {
        clear_current_user_failure();
    }
}

/// Overwrite the failure record with one that failed `age` ago, so a test can
/// sit either side of a backoff window without sleeping.
fn seed_current_user_failure(
    api_base: &str,
    token: &str,
    consecutive: u32,
    age: Duration,
    error: CurrentUserFetchError,
) {
    *CURRENT_USER_FAILURE.lock() = Some(CurrentUserFailure {
        api_base: api_base.to_string(),
        token: token.to_string(),
        failed_at: Instant::now()
            .checked_sub(age)
            .expect("test ages are far smaller than process uptime"),
        consecutive,
        error,
    });
}

#[test]
fn current_user_backoff_doubles_and_saturates_at_the_cap() {
    assert_eq!(current_user_backoff(1), CURRENT_USER_BACKOFF_BASE);
    assert_eq!(current_user_backoff(2), CURRENT_USER_BACKOFF_BASE * 2);
    assert_eq!(current_user_backoff(3), CURRENT_USER_BACKOFF_BASE * 4);
    assert_eq!(current_user_backoff(u32::MAX), CURRENT_USER_BACKOFF_MAX);
    // 0 is not a state the recorder can produce, but the function must not
    // answer it with a zero-length window.
    assert_eq!(current_user_backoff(0), CURRENT_USER_BACKOFF_BASE);

    let mut previous = Duration::ZERO;
    for consecutive in 1..=12 {
        let window = current_user_backoff(consecutive);
        assert!(
            window >= previous,
            "backoff must never narrow as failures accumulate: {consecutive} gave {window:?} after {previous:?}"
        );
        assert!(
            window <= CURRENT_USER_BACKOFF_MAX,
            "backoff must stay under the cap: {consecutive} gave {window:?}"
        );
        previous = window;
    }
}

#[test]
fn the_first_backoff_step_outlasts_both_the_fetch_timeout_and_the_poll() {
    // This is the property that actually stops the treadmill, and the one a
    // future constant change could silently break. A first step shorter than
    // the fetch timeout means the next poll finds the window already closed and
    // pays the full 5s again — which is the bug, not the fix. It must also
    // outlast the positive-cache TTL, because that TTL is what governs how soon
    // a poll asks for a live fetch at all.
    assert!(
        current_user_backoff(1) > AUTH_FETCH_TIMEOUT,
        "first backoff step {:?} must exceed the fetch timeout {:?}",
        current_user_backoff(1),
        AUTH_FETCH_TIMEOUT
    );
    assert!(
        current_user_backoff(1) > CURRENT_USER_REFRESH_TTL,
        "first backoff step {:?} must exceed the current-user cache TTL {:?}",
        current_user_backoff(1),
        CURRENT_USER_REFRESH_TTL
    );
}

#[test]
fn a_recorded_failure_suppresses_a_retry_inside_its_window() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    record_current_user_failure(
        "https://api.example.test",
        "token-a",
        CurrentUserFetchError::FetchFailed("request timed out after 5s".to_string()),
    );

    let (error, consecutive, remaining) =
        suppressed_current_user_failure("https://api.example.test", "token-a")
            .expect("a just-recorded failure must suppress the next attempt");
    assert_eq!(consecutive, 1);
    assert_eq!(error.message(), "request timed out after 5s");
    assert!(remaining <= CURRENT_USER_BACKOFF_BASE && !remaining.is_zero());
}

#[test]
fn a_recorded_failure_stops_suppressing_once_its_window_closes() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    seed_current_user_failure(
        "https://api.example.test",
        "token-a",
        1,
        CURRENT_USER_BACKOFF_BASE + Duration::from_millis(1),
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );

    assert!(
        suppressed_current_user_failure("https://api.example.test", "token-a").is_none(),
        "a failure older than its window must let the next attempt through"
    );
}

#[test]
fn consecutive_failures_widen_the_window() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    for _ in 0..3 {
        record_current_user_failure(
            "https://api.example.test",
            "token-a",
            CurrentUserFetchError::FetchFailed("boom".to_string()),
        );
    }

    let (_, consecutive, _) =
        suppressed_current_user_failure("https://api.example.test", "token-a")
            .expect("still inside the widened window");
    assert_eq!(consecutive, 3);

    // Three failures in, an attempt that would have been let through at the
    // first window is still suppressed.
    seed_current_user_failure(
        "https://api.example.test",
        "token-a",
        3,
        CURRENT_USER_BACKOFF_BASE + Duration::from_millis(1),
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );
    assert!(
        suppressed_current_user_failure("https://api.example.test", "token-a").is_some(),
        "the third failure's window must outlast the first failure's"
    );
}

#[test]
fn a_rejected_credential_is_never_recorded() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    record_current_user_failure(
        "https://api.example.test",
        "token-a",
        CurrentUserFetchError::Rejected("401 Unauthorized".to_string()),
    );

    // Replaying a rejection from a cache would either delay the deferred-session
    // cleanup the snapshot caller drives off that variant, or hand it a
    // different variant than the backend produced.
    assert!(
        suppressed_current_user_failure("https://api.example.test", "token-a").is_none(),
        "an auth rejection must not be backed off"
    );
}

#[test]
fn a_different_token_or_backend_bypasses_the_record() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    record_current_user_failure(
        "https://api.example.test",
        "token-a",
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );

    assert!(
        suppressed_current_user_failure("https://api.example.test", "token-b").is_none(),
        "signing in as someone else must not inherit the previous session's outage"
    );
    assert!(
        suppressed_current_user_failure("https://other.example.test", "token-a").is_none(),
        "switching environment must not inherit the previous backend's outage"
    );
    // …and the run it was recorded against is untouched by those probes.
    assert!(suppressed_current_user_failure("https://api.example.test", "token-a").is_some());
}

#[test]
fn clearing_the_record_lets_the_next_attempt_through() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    record_current_user_failure(
        "https://api.example.test",
        "token-a",
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );
    assert!(suppressed_current_user_failure("https://api.example.test", "token-a").is_some());

    // What sign-out and every success both call. Missing either is the failure
    // mode that matters: a record outliving its cause strands the app on the
    // stored snapshot after the backend is back.
    clear_current_user_failure();

    assert!(
        suppressed_current_user_failure("https://api.example.test", "token-a").is_none(),
        "a cleared record must not keep suppressing"
    );
}

#[tokio::test]
async fn fetch_current_user_cached_replays_a_recorded_failure_without_calling_the_backend() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    let mut config = Config::default();
    // A closed loopback port. Nothing here should reach it — the point of the
    // test is that the recorded failure short-circuits first — but if the probe
    // is removed the call fails locally with a connection error instead of
    // reaching out to the real backend.
    config.api_url = Some("http://127.0.0.1:9/".to_string());
    let api_base = current_user_api_base(&config);
    assert!(
        api_base.starts_with("http://127.0.0.1:9"),
        "precondition: the override must survive backend-url resolution, got {api_base}; \
         otherwise this test would talk to a real backend"
    );

    let token = "token-a";
    seed_current_user_failure(
        &api_base,
        token,
        1,
        Duration::from_millis(1),
        CurrentUserFetchError::FetchFailed("seeded outage marker".to_string()),
    );

    let error = fetch_current_user_cached(&config, token, true, current_user_generation())
        .await
        .expect_err("a recorded failure inside its window must be replayed");

    assert_eq!(
        error.message(),
        "seeded outage marker",
        "the fetch must replay the recorded failure rather than issue a request"
    );
}

#[path = "ops_signout_cache_tests.rs"]
mod signout_cache_tests;
