use super::*;
use crate::session::client::ClientHeaders;
use crate::session::test_support::{me_user, Backend, MeAnswer, LIVE_JWT};
use serde_json::json;

fn client(backend: &Backend) -> SessionClient {
    SessionClient::new(&backend.url, &ClientHeaders::new("openhuman")).unwrap()
}

#[test]
fn fetch_timeout_parsing_clamps_to_range() {
    assert_eq!(parse_fetch_timeout_secs(None), DEFAULT_FETCH_TIMEOUT_SECS);
    assert_eq!(parse_fetch_timeout_secs(Some(" 8 ")), 8);
    assert_eq!(
        parse_fetch_timeout_secs(Some("1")),
        DEFAULT_FETCH_TIMEOUT_SECS
    );
    assert_eq!(
        parse_fetch_timeout_secs(Some("13")),
        DEFAULT_FETCH_TIMEOUT_SECS
    );
    assert_eq!(
        parse_fetch_timeout_secs(Some("abc")),
        DEFAULT_FETCH_TIMEOUT_SECS
    );
}

#[test]
fn backoff_doubles_from_a_floor_and_saturates() {
    let t = Duration::from_secs(5);
    assert_eq!(backoff_base_for(t), Duration::from_secs(10));
    assert_eq!(
        backoff_base_for(Duration::from_secs(12)),
        Duration::from_secs(24)
    );
    assert_eq!(backoff_for(0, t), Duration::from_secs(10));
    assert_eq!(backoff_for(1, t), Duration::from_secs(10));
    assert_eq!(backoff_for(2, t), Duration::from_secs(20));
    assert_eq!(backoff_for(3, t), Duration::from_secs(40));
    assert_eq!(backoff_for(4, t), BACKOFF_MAX);
    assert_eq!(backoff_for(40, t), BACKOFF_MAX);
}

#[tokio::test]
async fn fresh_entry_is_served_without_a_request() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let first = cache
        .get_or_refresh(&client(&backend), &cred, false)
        .await
        .unwrap();
    assert_eq!(first.user.as_ref().unwrap()["_id"], "user-123");
    assert!(!first.stale);
    assert_eq!(first.stale_seconds, Some(0));
    let second = cache
        .get_or_refresh(&client(&backend), &cred, false)
        .await
        .unwrap();
    assert_eq!(second.user, first.user);
    assert_eq!(backend.me_calls(), 1);
    assert_eq!(cache.peek(), first.user);
}

#[tokio::test]
async fn force_bypasses_the_cache() {
    let backend = Backend::start(vec![
        MeAnswer::Ok(me_user()),
        MeAnswer::Ok(json!({ "_id": "user-123", "name": "Renamed" })),
    ])
    .await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    cache
        .get_or_refresh(&client(&backend), &cred, false)
        .await
        .unwrap();
    let forced = cache
        .get_or_refresh(&client(&backend), &cred, true)
        .await
        .unwrap();
    assert_eq!(forced.user.unwrap()["name"], "Renamed");
    assert_eq!(backend.me_calls(), 2);
}

#[tokio::test]
async fn concurrent_cache_misses_share_one_refresh() {
    let backend = Backend::start(vec![MeAnswer::Slow(50)]).await;
    let cache = CurrentUserCache::new();
    let credential = Credential::session(LIVE_JWT.as_str());
    let client = client(&backend);

    let (first, second) = tokio::join!(
        cache.get_or_refresh(&client, &credential, false),
        cache.get_or_refresh(&client, &credential, false),
    );

    assert_eq!(first.unwrap().user, second.unwrap().user);
    assert_eq!(
        backend.me_calls(),
        1,
        "cache misses must share one /auth/me request"
    );
}

// #6318 — a caller that queues behind a cold-cache refresh must observe the
// backoff window that refresh's *failure* just opened, not repeat the same
// doomed request. Before the fix, a waiter rechecked only the positive cache
// after acquiring `inflight`, so a failed first refresh sent every waiter to
// the network again.
#[tokio::test]
async fn a_failed_first_refresh_suppresses_the_waiter_queued_behind_it() {
    let backend = Backend::start(vec![MeAnswer::SlowStatus(50, 503), MeAnswer::Status(503)]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let c = client(&backend);

    let (first, second) = tokio::join!(
        cache.get_or_refresh(&c, &cred, false),
        cache.get_or_refresh(&c, &cred, false),
    );

    // Whichever caller lost the race to own the refresh must see the outage
    // too (never a silent success), but must not have paid for its own
    // request — the point of coalescing behind `inflight` at all.
    for outcome in [first, second] {
        assert!(
            matches!(
                outcome,
                Err(FetchMeError::Transient(_)) | Err(FetchMeError::Suppressed { .. })
            ),
            "expected an outage error, got {outcome:?}"
        );
    }
    assert_eq!(
        backend.me_calls(),
        1,
        "the caller queued behind the failed refresh must not issue a second request"
    );
}

// #6318 (review follow-up) — a positive entry that is stale-while-revalidate
// eligible (past `REFRESH_TTL`) must still be served when a background
// refresh has already recorded an availability failure for the same key;
// pre-empting it with the suppressed error for the whole backoff window
// would throw away perfectly good cached data. A rejection (see
// `rejection_is_retained_until_the_owner_observes_it`) is the one case that
// must still win over a stale cache — this test's failure is a plain
// availability outage, not a rejection.
#[tokio::test]
async fn stale_cache_is_served_through_an_availability_backoff_window() {
    let backend = Backend::start(vec![MeAnswer::Status(503)]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let c = client(&backend);
    let key = CurrentUserCache::key(&c, &cred);

    // A positive entry older than REFRESH_TTL, plus the failure a background
    // refresh for it would have recorded — as if `spawn_refresh` had already
    // run once and hit the outage.
    {
        let mut state = cache.lock();
        state.positive = Some(Positive {
            key: key.clone(),
            fetched_at: Instant::now() - REFRESH_TTL - Duration::from_secs(1),
            user: me_user(),
        });
        state.failure = Some(Failure {
            key: key.clone(),
            failed_at: Instant::now(),
            consecutive: 1,
            error: FetchMeError::Transient("boom".to_string()),
        });
    }

    let result = cache
        .get_or_refresh(&c, &cred, false)
        .await
        .expect("a stale positive entry must still be served during an availability outage");
    assert_eq!(result.user.as_ref().unwrap()["_id"], "user-123");
    assert!(result.stale, "the entry is past its outage-free TTL");
}

#[tokio::test]
async fn availability_failure_opens_a_backoff_window() {
    let backend = Backend::start(vec![MeAnswer::Status(503)]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let c = client(&backend);
    assert!(matches!(
        cache.get_or_refresh(&c, &cred, false).await,
        Err(FetchMeError::Transient(_))
    ));
    match cache.get_or_refresh(&c, &cred, false).await {
        Err(FetchMeError::Suppressed { consecutive, .. }) => assert_eq!(consecutive, 1),
        other => panic!("expected suppressed replay, got {other:?}"),
    }
    assert_eq!(backend.me_calls(), 1, "the replay must not hit the network");

    // A different identity is not suppressed by this one's outage.
    let other = Credential::session("other-token");
    assert!(matches!(
        cache.get_or_refresh(&c, &other, false).await,
        Err(FetchMeError::Transient(_))
    ));
    assert_eq!(backend.me_calls(), 2);

    // `force` ignores the window, and `forget` clears it.
    assert!(matches!(
        cache.get_or_refresh(&c, &cred, true).await,
        Err(FetchMeError::Transient(_))
    ));
    cache.forget();
    assert!(matches!(
        cache.get_or_refresh(&c, &cred, false).await,
        Err(FetchMeError::Transient(_))
    ));
    assert_eq!(backend.me_calls(), 4);
}

#[tokio::test]
async fn rejection_is_retained_until_the_owner_observes_it() {
    let backend = Backend::start(vec![MeAnswer::Status(401)]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let c = client(&backend);
    for _ in 0..2 {
        assert!(matches!(
            cache.get_or_refresh(&c, &cred, false).await,
            Err(FetchMeError::Rejected(_))
        ));
    }
    assert_eq!(backend.me_calls(), 1);
}

#[tokio::test]
async fn success_clears_the_failure_and_stamps_freshness() {
    let backend = Backend::start(vec![MeAnswer::Status(503), MeAnswer::Ok(me_user())]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let c = client(&backend);
    let _ = cache.get_or_refresh(&c, &cred, false).await;
    let ok = cache.get_or_refresh(&c, &cred, true).await.unwrap();
    assert!(!ok.stale);
    assert_eq!(ok.stale_seconds, Some(0));
    let cached = cache.get_or_refresh(&c, &cred, false).await.unwrap();
    assert!(!cached.stale);
    assert_eq!(backend.me_calls(), 2);
}

#[tokio::test]
async fn forget_drops_the_positive_entry() {
    let backend = Backend::start(vec![MeAnswer::Ok(me_user())]).await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    cache
        .get_or_refresh(&client(&backend), &cred, false)
        .await
        .unwrap();
    cache.forget();
    assert_eq!(cache.peek(), None);
    cache
        .get_or_refresh(&client(&backend), &cred, false)
        .await
        .unwrap();
    assert_eq!(backend.me_calls(), 2);
}

/// Intended behaviour, pinned: a rejection is authoritative only inside its
/// backoff window. Once the window has passed, the positive entry from before
/// the rejection is served stale while exactly one background refresh asks the
/// backend again, and that refresh's rejection is what the next read returns.
/// Sign-out can be one poll late; the backend is never asked twice per window.
#[tokio::test]
async fn a_lapsed_rejection_window_re_asks_the_backend_once_before_rejecting_again() {
    let backend = Backend::start(vec![
        MeAnswer::Ok(me_user()),
        MeAnswer::Status(401),
        MeAnswer::Status(401),
    ])
    .await;
    let cache = CurrentUserCache::new();
    let cred = Credential::session(LIVE_JWT.as_str());
    let c = client(&backend);
    cache.get_or_refresh(&c, &cred, false).await.unwrap();
    assert!(matches!(
        cache.get_or_refresh(&c, &cred, true).await,
        Err(FetchMeError::Rejected(_))
    ));
    assert!(matches!(
        cache.get_or_refresh(&c, &cred, false).await,
        Err(FetchMeError::Rejected(_))
    ));
    assert_eq!(backend.me_calls(), 2);

    // Let the window lapse (the positive entry, older still, is past its TTL).
    {
        let mut state = cache.lock();
        let failure = state.failure.as_mut().expect("recorded rejection");
        let lapse = backoff_for(failure.consecutive, fetch_timeout());
        failure.failed_at -= lapse;
        state.positive.as_mut().expect("positive entry").fetched_at -= lapse;
    }

    let served = cache.get_or_refresh(&c, &cred, false).await.unwrap();
    assert!(served.user.is_some());
    assert!(served.stale);
    for _ in 0..200 {
        if backend.me_calls() == 3 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(backend.me_calls(), 3);
    for _ in 0..200 {
        if cache
            .suppressed(&CurrentUserCache::key(&c, &cred))
            .is_some()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(matches!(
        cache.get_or_refresh(&c, &cred, false).await,
        Err(FetchMeError::Rejected(_))
    ));
    assert_eq!(backend.me_calls(), 3);
}
