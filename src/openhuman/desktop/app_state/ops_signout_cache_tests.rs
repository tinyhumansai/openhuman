// Sign-out invalidation for the current-user caches (#5758).
//
// `super` is the `tests` module; its `use super::*` re-exports both the ops
// items under test and this suite's shared cache lock.
use super::*;

// ── Sign-out cache invalidation (#5758) ────────────────────────────────────
//
// Both caches are keyed on `(api_base, token)`, so leaving either behind lets a
// re-login with the same JWT replay pre-logout state: the stale `/auth/me`
// snapshot from the positive cache, or the stale error from the negative one.
//
// `forget_current_user_caches` touches BOTH globals, so both cases hold both of
// this suite's cache locks — in this order, consistently, since no other test
// takes more than one.

// Sign-out itself must reach `forget_current_user_caches`, not merely be able
// to. The two direct-call cases below cannot see that wiring: deleting the call
// from `clear_session` leaves every test in this file, and in the credentials
// suite, passing. This one goes through the real entry point.
//
// `clear_session` clears the active-user marker under
// `default_root_openhuman_dir()`, which is derived from the process-global HOME,
// so it takes the shared env lock and pins HOME to its own tempdir -- the same
// precaution `clear_session_on_empty_store_reports_removed_false` documents.
#[tokio::test]
async fn signing_out_forgets_both_current_user_caches() {
    let _env_lock = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    let tmp = tempdir().expect("tempdir");
    let previous_home = std::env::var_os("HOME");
    unsafe { std::env::set_var("HOME", tmp.path()) };

    let api_base = "https://api.example.test";
    let token = "jwt-5758-signout";
    *CURRENT_USER_CACHE.lock() = Some(CachedCurrentUser {
        api_base: api_base.into(),
        token: token.into(),
        fetched_at: Instant::now(),
        user: json!({ "userId": "user-5758" }),
    });
    seed_current_user_failure(
        api_base,
        token,
        1,
        Duration::from_millis(0),
        CurrentUserFetchError::FetchFailed("backend unreachable".to_string()),
    );
    assert!(
        CURRENT_USER_CACHE.lock().is_some()
            && suppressed_current_user_failure(api_base, token).is_some(),
        "precondition: both caches must be populated"
    );

    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    crate::openhuman::security::credentials::ops::clear_session(&config)
        .await
        .expect("clear_session on an empty store still succeeds");

    match previous_home {
        Some(value) => unsafe { std::env::set_var("HOME", value) },
        None => unsafe { std::env::remove_var("HOME") },
    }

    assert!(
        CURRENT_USER_CACHE.lock().is_none(),
        "sign-out left the pre-logout /auth/me snapshot behind"
    );
    assert!(
        suppressed_current_user_failure(api_base, token).is_none(),
        "sign-out left the pre-logout availability failure behind"
    );
}

#[test]
fn forget_current_user_caches_clears_the_positive_snapshot() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    *CURRENT_USER_CACHE.lock() = Some(CachedCurrentUser {
        api_base: "https://api.example.test".into(),
        token: "jwt-5758".into(),
        fetched_at: Instant::now(),
        user: json!({ "userId": "user-5758" }),
    });
    assert!(
        CURRENT_USER_CACHE.lock().is_some(),
        "precondition: the snapshot must be cached for this test to mean anything"
    );

    forget_current_user_caches();

    assert!(
        CURRENT_USER_CACHE.lock().is_none(),
        "the pre-logout /auth/me snapshot survived sign-out"
    );
}

#[test]
fn forget_current_user_caches_clears_the_negative_failure() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    let api_base = "https://api.example.test";
    let token = "jwt-5758b";
    seed_current_user_failure(
        api_base,
        token,
        1,
        Duration::from_millis(0),
        CurrentUserFetchError::FetchFailed("backend unreachable".to_string()),
    );
    assert!(
        suppressed_current_user_failure(api_base, token).is_some(),
        "precondition: the failure must be suppressing for this test to mean anything"
    );

    forget_current_user_caches();

    assert!(
        suppressed_current_user_failure(api_base, token).is_none(),
        "the pre-logout availability failure survived sign-out, so a re-login with          the same JWT would replay it"
    );
}

// ── A refresh that races sign-out must not republish (#5758, review) ────────
//
// Clearing the two statics is not sufficient on its own. `fetch_current_user_cached`
// awaits the network between reading the caches and writing them, so a refresh
// already in flight when sign-out lands would write the pre-logout answer back
// afterwards — restoring exactly what sign-out just dropped.
//
// The synchronisation here is structural, not timed: the fake backend accepts
// the connection and then *waits*, so the request is provably in flight when
// sign-out runs, and the response is not released until after it.

/// Minimal `/auth/me` backend that hands control back to the test mid-request.
///
/// `Connection: close` rather than a Content-Length, so the body length is not
/// a thing this test can get subtly wrong.
async fn spawn_racing_backend(
    body: &'static str,
    status_line: &'static str,
) -> (
    u16,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let port = listener.local_addr().expect("local addr").port();

    let (in_flight_tx, in_flight_rx) = tokio::sync::oneshot::channel();
    let (release_tx, release_rx) = tokio::sync::oneshot::channel::<()>();

    let handle = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");

        // Drain enough of the request that the client has certainly sent it.
        let mut buf = [0u8; 1024];
        let _ = socket.read(&mut buf).await;

        // The request is in flight. Sign-out can happen now.
        let _ = in_flight_tx.send(());
        let _ = release_rx.await;

        let response = format!("{status_line}\r\nConnection: close\r\n\r\n{body}");
        let _ = socket.write_all(response.as_bytes()).await;
        let _ = socket.shutdown().await;
    });

    (port, in_flight_rx, release_tx, handle)
}

#[tokio::test]
async fn a_successful_refresh_that_races_sign_out_does_not_repopulate_the_snapshot() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    forget_current_user_caches();

    let (port, in_flight, release, server) = spawn_racing_backend(
        r#"{"data":{"userId":"user-before-logout"}}"#,
        "HTTP/1.1 200 OK",
    )
    .await;

    let mut config = Config::default();
    config.api_url = Some(format!("http://127.0.0.1:{port}/"));
    // Read where the caller reads it: alongside the token, before the refresh.
    let generation = current_user_generation();

    let fetch = tokio::spawn(async move {
        fetch_current_user_cached(&config, "jwt-race", true, generation).await
    });

    in_flight.await.expect("backend saw the request");
    // The user signs out while the refresh is on the wire.
    forget_current_user_caches();
    let _ = release.send(());

    let result = fetch.await.expect("join fetch");
    server.await.expect("join backend");

    assert!(
        result.is_ok(),
        "the caller that asked before sign-out still gets its answer"
    );
    assert!(
        CURRENT_USER_CACHE.lock().is_none(),
        "a refresh that finished after sign-out republished the pre-logout snapshot, \
         which is the state sign-out exists to drop"
    );
}

#[tokio::test]
async fn a_failed_refresh_that_races_sign_out_does_not_record_a_failure() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    forget_current_user_caches();

    // 500 is in AUTH_ME_REVALIDATION_TRANSIENT_STATUSES, so this takes the
    // record-a-failure path rather than the rejection path.
    let (port, in_flight, release, server) =
        spawn_racing_backend("backend is down", "HTTP/1.1 500 Internal Server Error").await;

    let mut config = Config::default();
    config.api_url = Some(format!("http://127.0.0.1:{port}/"));
    let api_base = current_user_api_base(&config);
    // Read where the caller reads it: alongside the token, before the refresh.
    let generation = current_user_generation();

    let fetch = tokio::spawn(async move {
        fetch_current_user_cached(&config, "jwt-race-err", true, generation).await
    });

    in_flight.await.expect("backend saw the request");
    forget_current_user_caches();
    let _ = release.send(());

    let result = fetch.await.expect("join fetch");
    server.await.expect("join backend");

    assert!(result.is_err(), "precondition: the fetch must have failed");
    assert!(
        suppressed_current_user_failure(&api_base, "jwt-race-err").is_none(),
        "a failure recorded after sign-out would suppress the first poll of the \
         next session, replaying an outage the new session never saw"
    );
}

// The two above prove the guard across a real await. These two close the
// narrower window inside it: the moment between reading the generation and
// taking the lock. There is nothing to pause here — the "pause" IS the call
// order, which is why these are deterministic rather than timed.

#[tokio::test]
async fn a_sign_out_landing_after_the_generation_check_still_wins_the_snapshot() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    // Both globals are touched here, so both guards are needed. The failure
    // lock is async, which is why these three were sync-only and unguarded.
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    forget_current_user_caches();
    // The refresh reads the generation, and is descheduled right here.
    let generation = current_user_generation();
    // Sign-out lands in that gap: it bumps, then clears both records.
    forget_current_user_caches();
    // The refresh resumes and tries to publish.
    let published = publish_current_user_unless_stale(
        generation,
        "http://127.0.0.1:1/",
        "jwt-before-logout",
        Some(json!({ "userId": "user-before-logout" })),
    );

    assert!(!published, "the publish must report that sign-out won");
    assert!(
        CURRENT_USER_CACHE.lock().is_none(),
        "checking the generation before taking the cache lock leaves a window in \
         which sign-out lands and the pre-logout snapshot is written anyway"
    );
}

#[tokio::test]
async fn a_sign_out_landing_after_the_generation_check_still_wins_the_failure_record() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    // Both globals are touched here, so both guards are needed. The failure
    // lock is async, which is why these three were sync-only and unguarded.
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    forget_current_user_caches();
    let generation = current_user_generation();
    forget_current_user_caches();
    let recorded = record_current_user_failure_unless_stale(
        generation,
        "http://127.0.0.1:1/",
        "jwt-before-logout",
        CurrentUserFetchError::FetchFailed("backend is down".to_string()),
    );

    assert!(!recorded, "the record must report that sign-out won");
    assert!(
        CURRENT_USER_FAILURE.lock().is_none(),
        "an outage recorded into that same window outlives the session it \
         belonged to, and suppresses the next session's first poll"
    );
}

#[tokio::test]
async fn publishing_under_the_current_generation_still_clears_a_recorded_outage() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    // Both globals are touched here, so both guards are needed. The failure
    // lock is async, which is why these three were sync-only and unguarded.
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    forget_current_user_caches();
    let generation = current_user_generation();
    record_current_user_failure(
        "http://127.0.0.1:1/",
        "jwt-live",
        CurrentUserFetchError::FetchFailed("backend was down".to_string()),
    );

    let published = publish_current_user_unless_stale(
        generation,
        "http://127.0.0.1:1/",
        "jwt-live",
        Some(json!({ "userId": "user-live" })),
    );

    assert!(
        published,
        "no sign-out happened; the answer must be published"
    );
    assert!(
        CURRENT_USER_CACHE.lock().is_some(),
        "the fresh snapshot must reach the positive cache"
    );
    assert!(
        CURRENT_USER_FAILURE.lock().is_none(),
        "a success must retire the outage, or the backoff outlives its cause"
    );
}

/// Sign-out between the profile load and the refresh must still win.
///
/// This is the window the generation moved to cover. `snapshot` reads the token
/// out of the auth profile — a load that busy-waits up to ~35s on a contended
/// lock — and only then starts the refresh. Reading the generation inside the
/// refresh meant a sign-out landing in that gap was already counted, so the
/// refresh compared the new generation against itself, passed, and published an
/// answer it had fetched with the token from before the sign-out.
///
/// Deterministic without a sleep: the sign-out is expressed by call order, and
/// the generation handed to the refresh is the one read before it.
#[tokio::test]
async fn a_sign_out_between_the_token_load_and_the_refresh_still_wins() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock();
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    forget_current_user_caches();

    // The snapshot reads the token, and the generation alongside it.
    let generation = current_user_generation();

    // The user signs out while the auth profile lock is still being waited on.
    forget_current_user_caches();

    let (port, in_flight, release, server) = spawn_racing_backend(
        r#"{"data":{"userId":"user-before-logout"}}"#,
        "HTTP/1.1 200 OK",
    )
    .await;

    let mut config = Config::default();
    config.api_url = Some(format!("http://127.0.0.1:{port}/"));

    // Only now does the refresh start, still carrying the pre-sign-out token.
    let fetch = tokio::spawn(async move {
        fetch_current_user_cached(&config, "jwt-before-logout", true, generation).await
    });

    in_flight.await.expect("backend saw the request");
    let _ = release.send(());

    let result = fetch.await.expect("join fetch");
    server.await.expect("join backend");

    assert!(result.is_ok(), "the caller still gets its answer");
    assert!(
        CURRENT_USER_CACHE.lock().is_none(),
        "a refresh holding the pre-sign-out token republished the identity that \
         sign-out exists to drop, because it read the generation after the \
         sign-out rather than alongside the token"
    );
}
