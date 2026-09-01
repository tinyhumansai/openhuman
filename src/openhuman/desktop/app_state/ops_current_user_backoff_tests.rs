//! The current-user failure backoff, its replay, and the staleness it reports.
//!
//! Split out of `ops_tests.rs` for the layout gate. Kept as one file because
//! the three subjects are one mechanism: a failure is recorded, replayed while
//! its window is open (#5930), and surfaced as snapshot staleness — and the
//! helpers that seed an outage are shared across all three.

use super::*;
use once_cell::sync::Lazy as TestLazy;
use serde_json::json;

// ── Current-user failure backoff (#5624) ────────────────────────────────────
//
// While the backend is unreachable, every `app_state_snapshot` poll used to
// re-attempt `auth_get_me` and re-pay the full `auth_fetch_timeout`, because a
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
pub(super) static CURRENT_USER_FAILURE_TEST_LOCK: TestLazy<tokio::sync::Mutex<()>> =
    TestLazy::new(|| tokio::sync::Mutex::new(()));

/// Drops the seeded outage on the way out, so one test cannot leak into the next.
pub(super) struct CurrentUserFailureResetGuard;

impl Drop for CurrentUserFailureResetGuard {
    fn drop(&mut self) {
        clear_current_user_failure();
    }
}

/// Overwrite the failure record with one that failed `age` ago, so a test can
/// sit either side of a backoff window without sleeping.
pub(super) fn seed_current_user_failure(
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
    let base = current_user_backoff_base();
    assert_eq!(current_user_backoff(1), base);
    assert_eq!(current_user_backoff(2), base * 2);
    assert_eq!(current_user_backoff(3), base * 4);
    assert_eq!(current_user_backoff(u32::MAX), CURRENT_USER_BACKOFF_MAX);
    // 0 is not a state the recorder can produce, but the function must not
    // answer it with a zero-length window.
    assert_eq!(current_user_backoff(0), base);

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
        current_user_backoff(1) > auth_fetch_timeout(),
        "first backoff step {:?} must exceed the fetch timeout {:?}",
        current_user_backoff(1),
        auth_fetch_timeout()
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
    assert!(remaining <= current_user_backoff_base() && !remaining.is_zero());
}

#[test]
fn a_recorded_failure_stops_suppressing_once_its_window_closes() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    seed_current_user_failure(
        "https://api.example.test",
        "token-a",
        1,
        current_user_backoff_base() + Duration::from_millis(1),
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
        current_user_backoff_base() + Duration::from_millis(1),
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

// ── Replay is distinguishable from a live failure (#5930) ───────────────────
//
// #5930 was filed on four `WRN … current user refresh failed … request timed
// out after 5s` lines seconds apart, read as the app hammering a failing
// endpoint. They were replays of one recorded failure — microseconds each, no
// request made — because the suppression path returned the recorded error
// verbatim and the snapshot caller had no way to tell the two apart. These pin
// the distinction so the same false alarm cannot be filed twice.

#[tokio::test]
async fn a_replayed_failure_is_tagged_suppressed_and_keeps_the_original_message() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.lock().await;
    let _reset = CurrentUserFailureResetGuard;

    let mut config = Config::default();
    config.api_url = Some("http://127.0.0.1:9/".to_string());
    let api_base = current_user_api_base(&config);
    let token = "token-a";

    seed_current_user_failure(
        &api_base,
        token,
        3,
        Duration::from_millis(1),
        CurrentUserFetchError::FetchFailed("request timed out after 5s".to_string()),
    );

    let error = fetch_current_user_cached(&config, token, true, current_user_generation())
        .await
        .expect_err("a recorded failure inside its window must be replayed");

    match &error {
        CurrentUserFetchError::Suppressed {
            inner,
            consecutive,
            retry_in,
        } => {
            assert_eq!(*consecutive, 3, "the run length must survive the replay");
            assert!(
                *retry_in > Duration::ZERO && *retry_in <= current_user_backoff(3),
                "remaining window {retry_in:?} must be inside the step for 3 failures"
            );
            assert!(
                matches!(**inner, CurrentUserFetchError::FetchFailed(_)),
                "the original variant must be preserved, not flattened"
            );
        }
        other => panic!("expected a Suppressed replay, got {other:?}"),
    }

    assert_eq!(
        error.message(),
        "request timed out after 5s",
        "callers reading only the message must be unaffected by the wrapper"
    );
}

#[test]
fn a_suppressed_replay_is_never_recorded_as_a_fresh_failure() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserFailureResetGuard;

    let suppressed = CurrentUserFetchError::Suppressed {
        inner: Box::new(CurrentUserFetchError::FetchFailed("boom".to_string())),
        consecutive: 2,
        retry_in: Duration::from_secs(7),
    };
    assert!(
        !suppressed.is_availability_failure(),
        "a replay is not a new observation; recording it would widen the window \
         on evidence the backend never supplied"
    );

    record_current_user_failure("https://api.example.test", "token-a", suppressed);
    assert!(
        suppressed_current_user_failure("https://api.example.test", "token-a").is_none(),
        "recording a replay must be a no-op"
    );
}

// ── Stale-snapshot age (#5930) ──────────────────────────────────────────────

/// Drops both the outage and the success stamp on the way out.
pub(super) struct CurrentUserStalenessResetGuard;

impl Drop for CurrentUserStalenessResetGuard {
    fn drop(&mut self) {
        clear_current_user_failure();
        clear_current_user_success();
    }
}

#[test]
fn staleness_is_reported_only_for_the_identity_that_actually_failed() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserStalenessResetGuard;
    clear_current_user_failure();
    clear_current_user_success();

    let api_base = "https://api.example.test";

    let (stale, age) = current_user_staleness(api_base, "token-a");
    assert!(!stale, "no recorded failure means nothing is stale");
    assert_eq!(
        age, None,
        "no success this process means the age is unknown"
    );

    record_current_user_failure(
        api_base,
        "token-a",
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );

    assert!(
        current_user_staleness(api_base, "token-a").0,
        "the identity that failed is serving data it could not refresh"
    );
    assert!(
        !current_user_staleness(api_base, "token-b").0,
        "a different token must not inherit this identity's outage"
    );
    assert!(
        !current_user_staleness("https://other.example.test", "token-a").0,
        "a different backend must not inherit this environment's outage"
    );
}

#[test]
fn a_success_stamps_the_age_and_clears_the_stale_flag() {
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserStalenessResetGuard;
    clear_current_user_failure();
    clear_current_user_success();

    let api_base = "https://api.example.test";
    record_current_user_failure(
        api_base,
        "token-a",
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );
    assert!(current_user_staleness(api_base, "token-a").0);

    // What `fetch_current_user_cached` does on every successful answer.
    clear_current_user_failure();
    note_current_user_success(api_base, "token-a");

    let (stale, age) = current_user_staleness(api_base, "token-a");
    assert!(!stale, "a success must clear the stale flag");
    assert!(
        age.is_some(),
        "once the backend has answered, the age of the data is knowable"
    );

    // Sign-out must not leave the next account inheriting this one's freshness.
    clear_current_user_success();
    assert_eq!(current_user_staleness(api_base, "token-a").1, None);
}

#[test]
fn one_identitys_success_is_never_reported_as_anothers_age() {
    // The stale flag was keyed on `(api_base, token)` from the start but the
    // age was read from a bare global, so identity A succeeding and identity B
    // then failing reported A's freshness against B's outage. The ordinary
    // `credentials::clear_session` logout does not run the deferred-rejection
    // cleanup either, so the same leak survived a logout and re-login in one
    // process. Keying the stamp closes both.
    let _failure_lock = CURRENT_USER_FAILURE_TEST_LOCK.blocking_lock();
    let _reset = CurrentUserStalenessResetGuard;
    clear_current_user_failure();
    clear_current_user_success();

    let api_base = "https://api.example.test";
    note_current_user_success(api_base, "token-a");

    let (_, age_a) = current_user_staleness(api_base, "token-a");
    assert!(
        age_a.is_some(),
        "the identity that succeeded knows its own age"
    );

    let (_, age_b) = current_user_staleness(api_base, "token-b");
    assert_eq!(
        age_b, None,
        "a different token must not inherit another identity's freshness"
    );

    let (_, age_other_env) = current_user_staleness("https://other.example.test", "token-a");
    assert_eq!(
        age_other_env, None,
        "a different backend must not inherit another environment's freshness"
    );

    // The pairing that actually shipped the bug: B is stale, and must not be
    // handed A's age alongside that.
    record_current_user_failure(
        api_base,
        "token-b",
        CurrentUserFetchError::FetchFailed("boom".to_string()),
    );
    let (stale_b, age_b) = current_user_staleness(api_base, "token-b");
    assert!(stale_b, "B's outage is B's");
    assert_eq!(
        age_b, None,
        "B has no success of its own, so its age is unknown"
    );
}
