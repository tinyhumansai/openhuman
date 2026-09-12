//! What one `app_state_snapshot` poll costs, and what the core can stop paying.
//!
//! Split out of `ops_tests.rs` for the layout gate. Kept as one file because
//! both subjects are one measurement: #6180 reported 732 snapshot calls in
//! ~3.5h with no call under 500ms and spikes to 1910ms, and the two tests here
//! pin the two halves of that number the core controls.

use super::tests::APP_STATE_CACHE_TEST_LOCK;
use super::*;
use serde_json::json;
// Measured against the production backend over a ~250ms RTT link, one
// `GET /auth/me` costs ~380-540ms on a warm connection and ~780-1420ms on a
// cold one — and a 404 for a route that does not exist on the same host costs
// the same as `/auth/me`, so the floor is the round trip itself, not the
// endpoint. Neither number is something the core can shrink. What it can do is
// stop paying them: not block the poll on the round trip, and not open a new
// connection each time it makes one. One test each.

/// Serve the identity we already have; re-confirm it behind the poll.
///
/// `fetch_current_user_cached` used to block on `GET /auth/me` the moment its
/// entry aged past the TTL, which — with the frontend scheduling the next poll
/// from the previous *response* — was every poll. That put a floor of one WAN
/// round trip under every `app_state_snapshot`. The stub here answers slowly on
/// purpose: an expired entry must come back immediately anyway, and the refresh
/// must still land.
#[tokio::test]
async fn an_expired_current_user_is_served_while_it_refreshes() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock().await;
    // A successful refresh calls `clear_current_user_failure`, which wipes a
    // global the backoff tests seed.
    let _failure_lock = super::current_user_backoff_tests::CURRENT_USER_FAILURE_TEST_LOCK
        .lock()
        .await;
    struct CacheResetGuard;
    impl Drop for CacheResetGuard {
        fn drop(&mut self) {
            *CURRENT_USER_CACHE.lock() = None;
        }
    }
    let _reset = CacheResetGuard;

    const BACKEND_DELAY: Duration = Duration::from_millis(2000);

    let app = axum::Router::new().route(
        "/auth/me",
        axum::routing::get(|| async move {
            tokio::time::sleep(BACKEND_DELAY).await;
            axum::Json(json!({ "success": true, "data": { "firstName": "from-backend" } }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let config = Config {
        api_url: Some(format!("http://{addr}")),
        ..Config::default()
    };
    // The failure record is keyed on `(api_base, token)` and this base carries a
    // fresh ephemeral port, so no seeded outage can match it and none is seeded
    // here — nothing to clean up, and no cross-test write.
    *CURRENT_USER_CACHE.lock() = Some(CachedCurrentUser {
        api_base: current_user_api_base(&config),
        token: "tok".into(),
        fetched_at: Instant::now() - (CURRENT_USER_REFRESH_TTL + Duration::from_secs(1)),
        user: json!({ "firstName": "from-cache" }),
    });

    let started = Instant::now();
    let served = fetch_current_user_cached(&config, "tok", true, current_user_generation())
        .await
        .expect("an expired entry is still an answer");
    let waited = started.elapsed();

    assert_eq!(
        served.as_ref().and_then(|u| u.get("firstName")),
        Some(&json!("from-cache")),
        "the expired entry must be what the poll is handed"
    );
    assert!(
        waited < Duration::from_millis(400),
        "the poll waited {waited:?} on the backend; serving the entry it already \
         had is the whole point — this is #6180's per-poll round trip"
    );

    // ...and the refresh must actually happen, or "fast" is just "never
    // refreshes". Wait past the stub's delay for the cache to flip.
    let refreshed = tokio::time::timeout(BACKEND_DELAY * 3, async {
        loop {
            let landed = CURRENT_USER_CACHE
                .lock()
                .as_ref()
                .and_then(|entry| entry.user.get("firstName").cloned());
            if landed == Some(json!("from-backend")) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    assert!(
        refreshed.is_ok(),
        "the background refresh never replaced the stale entry; \
         serving stale forever is not stale-while-revalidate"
    );
}

/// Successive `/auth/me` fetches must share one TCP connection.
///
/// `fetch_current_user` built a fresh `reqwest::Client` per call, and a
/// `Client` owns its connection pool — so every fetch opened a new connection
/// and paid a TCP handshake plus a TLS handshake before the request could go
/// out. Over a WAN link that is two extra round trips on top of the one the
/// request costs, and it is the gap between #6180's ~500ms floor and its
/// ~1900ms spikes.
///
/// Asserted on the wire, because `reqwest::Client` exposes no pool statistics:
/// the server sees a distinct ephemeral source port for every new connection,
/// so a reused connection is exactly one distinct peer address across N
/// requests.
#[tokio::test]
async fn current_user_fetches_reuse_one_connection() {
    use axum::extract::ConnectInfo;
    use std::collections::BTreeSet;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex as StdMutex};

    type Peers = Arc<StdMutex<BTreeSet<SocketAddr>>>;
    let peers: Peers = Arc::new(StdMutex::new(BTreeSet::new()));
    let peers_for_route = peers.clone();

    let app = axum::Router::new().route(
        "/auth/me",
        axum::routing::get(move |ConnectInfo(peer): ConnectInfo<SocketAddr>| {
            let peers = peers_for_route.clone();
            async move {
                peers.lock().unwrap().insert(peer);
                axum::Json(json!({ "success": true, "data": { "firstName": "steven" } }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await;
    });

    let config = Config {
        api_url: Some(format!("http://{addr}")),
        ..Config::default()
    };

    // Sequential, like the poll loop: each call must find the previous call's
    // connection idle in the pool and reuse it.
    for _ in 0..3 {
        fetch_current_user(&config, "tok")
            .await
            .expect("the stub answers /auth/me");
    }

    let distinct = peers.lock().unwrap().len();
    assert_eq!(
        distinct, 1,
        "three sequential /auth/me fetches opened {distinct} connections; each new \
         one costs a TCP and a TLS handshake before the request goes out (#6180)"
    );
}

/// A detached refresh must not commit an identity the app has already left.
///
/// The background refresh outlives the poll that started it, so a logout and
/// re-login (or an environment switch) can land while it is still in flight.
/// Every write it then makes is process-global. The keyed reads in
/// `fetch_current_user_cached` would only miss on a regressed entry — but
/// `peek_cached_current_user_identity` reads that same slot with **no** key
/// check, to embed the user in the agent's prompts (#926), so a regressed entry
/// is served there as the current user.
#[tokio::test]
async fn a_background_refresh_does_not_overwrite_a_newer_identity() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock().await;
    let _failure_lock = super::current_user_backoff_tests::CURRENT_USER_FAILURE_TEST_LOCK
        .lock()
        .await;
    struct CacheResetGuard;
    impl Drop for CacheResetGuard {
        fn drop(&mut self) {
            *CURRENT_USER_CACHE.lock() = None;
        }
    }
    let _reset = CacheResetGuard;

    let app = axum::Router::new().route(
        "/auth/me",
        axum::routing::get(|| async {
            axum::Json(json!({ "success": true, "data": { "firstName": "signed-out-user" } }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let stale_config = Config {
        api_url: Some(format!("http://{addr}")),
        ..Config::default()
    };

    // The identity the app moved to while the refresh below was in flight.
    *CURRENT_USER_CACHE.lock() = Some(CachedCurrentUser {
        api_base: current_user_api_base(&stale_config),
        token: "token-for-the-user-who-just-signed-in".into(),
        fetched_at: Instant::now(),
        user: json!({ "firstName": "signed-in-user" }),
    });

    // The refresh the *previous* identity started, landing late.
    refresh_current_user_now(
        &stale_config,
        "token-for-the-user-who-signed-out",
        current_user_generation(),
        RefreshOrigin::Background,
    )
    .await
    .expect("the stub answers /auth/me");

    let cached = CURRENT_USER_CACHE
        .lock()
        .as_ref()
        .and_then(|entry| entry.user.get("firstName").cloned());
    assert_eq!(
        cached,
        Some(json!("signed-in-user")),
        "a background refresh for the previous identity overwrote the current one; \
         `peek_cached_current_user_identity` reads this slot unkeyed (#926)"
    );

    // The blocking path is authoritative by construction — its caller is still
    // awaiting it — so it must still commit.
    refresh_current_user_now(
        &stale_config,
        "token-for-the-user-who-signed-out",
        current_user_generation(),
        RefreshOrigin::Blocking,
    )
    .await
    .expect("the stub answers /auth/me");
    let cached = CURRENT_USER_CACHE
        .lock()
        .as_ref()
        .and_then(|entry| entry.user.get("firstName").cloned());
    assert_eq!(
        cached,
        Some(json!("signed-out-user")),
        "the blocking path must still commit; only detached refreshes are discarded"
    );
}

/// The TTL clock must start when the request goes out, not when it lands.
///
/// `CURRENT_USER_REFRESH_TTL` is measured against `fetched_at`, and the poll
/// loop schedules itself from the previous *response*. Stamping `fetched_at` at
/// completion folds the round trip into the next window: a refresh taking `L`
/// leaves the following poll only `TTL - L` from expiry, that poll reads the
/// entry as fresh, and the refresh after it never happens — the cadence halves
/// as a silent side effect of no longer blocking (#6190 review). Stamping at
/// initiation keeps the wall-clock cadence exactly what it was before.
#[tokio::test]
async fn the_refresh_ttl_clock_starts_when_the_request_goes_out() {
    let _cache_lock = APP_STATE_CACHE_TEST_LOCK.lock().await;
    let _failure_lock = super::current_user_backoff_tests::CURRENT_USER_FAILURE_TEST_LOCK
        .lock()
        .await;
    struct CacheResetGuard;
    impl Drop for CacheResetGuard {
        fn drop(&mut self) {
            *CURRENT_USER_CACHE.lock() = None;
        }
    }
    let _reset = CacheResetGuard;

    // Stands in for the WAN round trip the issue measured at ~380-540ms.
    const BACKEND_LATENCY: Duration = Duration::from_millis(600);

    let app = axum::Router::new().route(
        "/auth/me",
        axum::routing::get(|| async move {
            tokio::time::sleep(BACKEND_LATENCY).await;
            axum::Json(json!({ "success": true, "data": { "firstName": "steven" } }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });

    let config = Config {
        api_url: Some(format!("http://{addr}")),
        ..Config::default()
    };
    refresh_current_user_now(
        &config,
        "tok",
        current_user_generation(),
        RefreshOrigin::Blocking,
    )
    .await
    .expect("the stub answers /auth/me");

    let age = CURRENT_USER_CACHE
        .lock()
        .as_ref()
        .expect("the refresh committed an entry")
        .fetched_at
        .elapsed();

    assert!(
        age >= BACKEND_LATENCY,
        "the entry is {age:?} old immediately after a {BACKEND_LATENCY:?} fetch, so the clock \
         was started on the response: the round trip has been folded into the next TTL window \
         and the poll after next will skip its refresh (#6190)"
    );
}
