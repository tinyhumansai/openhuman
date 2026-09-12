use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[test]
fn budget_cache_get_returns_none_when_empty() {
    let cache = BudgetProbeCache::new();
    assert_eq!(cache.get(Instant::now(), Duration::from_secs(30)), None);
}

// ── GH #4153: `/teams/me/usage` failure backoff ──────────────────────────

use crate::core::observability::is_suppressed_usage_probe_backoff;

const TTL: Duration = Duration::from_secs(60);
const FAIL_KEY: &str = "backend-under-test";

#[test]
fn usage_failure_cache_freshness_window() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    assert!(
        !cache.is_fresh(FAIL_KEY, base, TTL),
        "empty cache is never fresh"
    );
    cache.record(FAIL_KEY, base);
    assert!(cache.is_fresh(FAIL_KEY, base + Duration::from_secs(5), TTL));
    assert!(!cache.is_fresh(FAIL_KEY, base + Duration::from_secs(61), TTL));
    cache.clear();
    assert!(
        !cache.is_fresh(FAIL_KEY, base, TTL),
        "cleared cache is never fresh"
    );
}

// GH #4153: a failure anchored under one backend key must NOT suppress a
// probe for a different backend (e.g. after the user fixes BACKEND_URL or
// the session re-points the backend) — otherwise the new route never gets
// tested for up to the backoff window.
#[test]
fn failure_backoff_is_keyed_per_backend() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    cache.record("https://old.example/api/v1", base);
    assert!(cache.is_fresh("https://old.example/api/v1", base, TTL));
    assert!(
        !cache.is_fresh("https://new.example/api/v1", base, TTL),
        "a different backend key must not inherit the old backend's backoff"
    );
}

// T1 — first failure of a streak hits the backend, reports verbatim, anchors.
#[tokio::test]
async fn first_failure_reports_and_anchors() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    let calls = AtomicUsize::new(0);
    let err = get_usage_with_cache(&cache, FAIL_KEY, TTL, base, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err("GET /teams/me/usage failed (500 Internal Server Error): ".to_string()) }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "backend probed once");
    assert!(
        !is_suppressed_usage_probe_backoff(&err),
        "first failure must NOT be the backoff sentinel (it reports): {err}"
    );
    assert!(cache.is_fresh(FAIL_KEY, base, TTL), "streak anchored");
}

// T2 — a repeat inside the window short-circuits WITHOUT touching the
// backend and returns the demote sentinel.
#[tokio::test]
async fn repeat_within_window_suppressed_and_skips_backend() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    cache.record(FAIL_KEY, base);
    let calls = AtomicUsize::new(0);
    let err = get_usage_with_cache(&cache, FAIL_KEY, TTL, base + Duration::from_secs(5), || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { panic!("fetch must not run inside the backoff window") }
    })
    .await
    .unwrap_err();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "backend NOT probed (backpressure)"
    );
    assert!(
        is_suppressed_usage_probe_backoff(&err),
        "repeat must carry the backoff sentinel: {err}"
    );
}

// T3 — once the window expires the backend is probed again (≤1 report/min).
#[tokio::test]
async fn window_expiry_reprobes_and_reports() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    cache.record(FAIL_KEY, base);
    let later = base + Duration::from_secs(61);
    let calls = AtomicUsize::new(0);
    let err = get_usage_with_cache(&cache, FAIL_KEY, TTL, later, || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Err("GET /teams/me/usage failed (500 Internal Server Error): ".to_string()) }
    })
    .await
    .unwrap_err();
    assert_eq!(calls.load(Ordering::SeqCst), 1, "stale window re-probes");
    assert!(
        !is_suppressed_usage_probe_backoff(&err),
        "re-probe failure reports"
    );
    assert!(
        cache.is_fresh(FAIL_KEY, later, TTL),
        "streak re-anchored at the new probe"
    );
}

// T4 — a success clears the streak so the next failure reports immediately.
#[tokio::test]
async fn success_clears_streak() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    cache.record(FAIL_KEY, base);
    let later = base + Duration::from_secs(61); // stale → fetch runs
    let outcome = get_usage_with_cache(&cache, FAIL_KEY, TTL, later, || async {
        Ok(serde_json::json!({"remainingUsd": 5.0}))
    })
    .await
    .expect("success");
    assert_eq!(outcome.value["remainingUsd"], 5.0);
    assert!(
        !cache.is_fresh(FAIL_KEY, later, TTL),
        "success cleared the streak"
    );
}

// T5 — session-expiry must flow verbatim and must NOT anchor the window
// (its own RPC arm drives auth recovery).
#[tokio::test]
async fn session_expiry_bypasses_backoff() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    let err = get_usage_with_cache(&cache, FAIL_KEY, TTL, base, || async {
        Err("SESSION_EXPIRED: backend rejected session token on GET /teams/me/usage".to_string())
    })
    .await
    .unwrap_err();
    assert!(
        err.contains("SESSION_EXPIRED"),
        "propagated verbatim: {err}"
    );
    assert!(!is_suppressed_usage_probe_backoff(&err));
    assert!(
        !cache.is_fresh(FAIL_KEY, base, TTL),
        "session-expiry must not start a backoff streak"
    );
}

// T6 — the producer's sentinel and the classifier are coupled (no drift).
#[tokio::test]
async fn produced_sentinel_is_classified() {
    let cache = UsageFailureCache::new();
    let base = Instant::now();
    cache.record(FAIL_KEY, base);
    let err = get_usage_with_cache(&cache, FAIL_KEY, TTL, base, || async {
        unreachable!("fresh window short-circuits")
    })
    .await
    .unwrap_err();
    assert!(
        err.starts_with(crate::core::observability::USAGE_PROBE_BACKOFF_PREFIX),
        "sentinel built from the shared prefix constant: {err}"
    );
    assert!(is_suppressed_usage_probe_backoff(&err));
}

#[test]
fn budget_cache_returns_value_within_ttl_and_expires_after() {
    let cache = BudgetProbeCache::new();
    let base = Instant::now();
    cache.put(base, true);
    // Within TTL → cached value.
    assert_eq!(
        cache.get(base + Duration::from_secs(5), Duration::from_secs(30)),
        Some(true)
    );
    // Past TTL → miss (caller must re-probe).
    assert_eq!(
        cache.get(base + Duration::from_secs(31), Duration::from_secs(30)),
        None
    );
}

#[tokio::test]
async fn cache_hit_skips_fetch() {
    let cache = BudgetProbeCache::new();
    cache.put(Instant::now(), true);
    let calls = AtomicUsize::new(0);
    let result = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Some(false) }
    })
    .await;
    assert!(result, "fresh cache value should be returned");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "fetch must be skipped on cache hit"
    );
}

#[tokio::test]
async fn cache_miss_fetches_and_caches() {
    let cache = BudgetProbeCache::new();
    let calls = AtomicUsize::new(0);
    // First call: empty cache → fetch runs and result is cached.
    let first = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Some(true) }
    })
    .await;
    assert!(first);
    // Second call: cache is now warm → fetch is skipped.
    let second = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Some(false) }
    })
    .await;
    assert!(
        second,
        "second call should return the cached true, not the fresh false"
    );
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "only the first call should fetch"
    );
}

#[tokio::test]
async fn failed_probe_is_not_cached_and_defers_to_backend() {
    let cache = BudgetProbeCache::new();
    let calls = AtomicUsize::new(0);
    // Probe failure (None) → returns false (defer to backend) and does not cache.
    let first = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { None }
    })
    .await;
    assert!(!first, "failed probe defers to backend (not exhausted)");
    // Next call must re-probe because the failure wasn't cached.
    let second = budget_exhausted_with_cache(&cache, Duration::from_secs(30), || {
        calls.fetch_add(1, Ordering::SeqCst);
        async { Some(true) }
    })
    .await;
    assert!(second);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "failed probe is re-fetched next time"
    );
}

#[test]
fn build_api_path_encodes_reserved_characters_in_segments() {
    let path = build_api_path(&["teams", "team/with?reserved", "members", "user#frag"])
        .expect("path should build");

    assert_eq!(path, "/teams/team%2Fwith%3Freserved/members/user%23frag");
}

#[test]
fn build_api_path_empty_segments_list_is_root() {
    let path = build_api_path(&[]).expect("path should build");
    assert_eq!(path, "/");
}

#[test]
fn build_api_path_preserves_segment_order() {
    let path = build_api_path(&["a", "b", "c"]).expect("path should build");
    assert_eq!(path, "/a/b/c");
}

#[test]
fn build_api_path_percent_encodes_spaces_and_unicode() {
    let path = build_api_path(&["teams", "with space", "👥"]).expect("path should build");
    assert!(path.contains("with%20space"));
    // Unicode must be percent-encoded (UTF-8 bytes).
    assert!(!path.contains('👥'));
}

#[test]
fn normalize_id_rejects_empty_with_field_name() {
    let err = normalize_id("", "teamId").unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[test]
fn normalize_id_rejects_whitespace_only() {
    let err = normalize_id("   \t\n", "userId").unwrap_err();
    assert_eq!(err, "userId is required");
}

#[test]
fn normalize_id_trims_and_keeps_body() {
    assert_eq!(normalize_id("  abc  ", "teamId").unwrap(), "abc");
}

#[test]
fn normalize_id_preserves_internal_whitespace() {
    // Only leading/trailing whitespace is stripped — interior is preserved
    // so we don't silently corrupt caller-provided identifiers.
    assert_eq!(normalize_id("a b", "x").unwrap(), "a b");
}

#[test]
fn usage_budget_exhausted_requires_real_cycle_signal() {
    assert!(!usage_budget_exhausted(&json!({
        "remainingUsd": 0,
        "cycleBudgetUsd": 0,
        "cycleSpentUsd": 0,
    })));
    assert!(usage_budget_exhausted(&json!({
        "remainingUsd": 0,
        "cycleBudgetUsd": 10,
        "cycleSpentUsd": 10,
    })));
    assert!(usage_budget_exhausted(&json!({
        "remainingUsd": 0,
        "cycleBudgetUsd": 0,
        "cycleSpentUsd": 2,
    })));
}

#[test]
fn usage_budget_exhausted_honors_remaining_and_bypass() {
    assert!(!usage_budget_exhausted(&json!({
        "remainingUsd": 0.25,
        "cycleBudgetUsd": 10,
    })));
    assert!(!usage_budget_exhausted(&json!({
        "remainingUsd": 0,
        "cycleBudgetUsd": 10,
        "bypassCycleLimit": true,
    })));
}

// --- pre-HTTP input validation (no network) -----------------------------

fn cfg() -> Config {
    Config::default()
}

#[tokio::test]
async fn list_members_rejects_empty_team_id() {
    let err = list_members(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn list_members_rejects_whitespace_team_id() {
    let err = list_members(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn get_team_rejects_empty_team_id() {
    let err = get_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn create_team_rejects_empty_name() {
    let err = create_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "name is required");
}

#[tokio::test]
async fn create_team_rejects_whitespace_name() {
    let err = create_team(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "name is required");
}

#[tokio::test]
async fn update_team_rejects_empty_team_id() {
    let err = update_team(&cfg(), "", Some("new")).await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn delete_team_rejects_empty_team_id() {
    let err = delete_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn switch_team_rejects_empty_team_id() {
    let err = switch_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn leave_team_rejects_empty_team_id() {
    let err = leave_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn join_team_rejects_empty_code() {
    let err = join_team(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "code is required");
}

#[tokio::test]
async fn join_team_rejects_whitespace_code() {
    let err = join_team(&cfg(), "   ").await.unwrap_err();
    assert_eq!(err, "code is required");
}

#[tokio::test]
async fn create_invite_rejects_empty_team_id() {
    let err = create_invite(&cfg(), "", None, None).await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn remove_member_validates_team_id_before_user_id() {
    // Failing input order must be deterministic: team_id is normalized
    // first, so an empty team_id reports the teamId error regardless of
    // the user_id.
    let err = remove_member(&cfg(), "", "someone").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn remove_member_rejects_empty_user_id_when_team_id_valid() {
    let err = remove_member(&cfg(), "t1", "").await.unwrap_err();
    assert_eq!(err, "userId is required");
}

#[tokio::test]
async fn change_member_role_rejects_missing_role() {
    let err = change_member_role(&cfg(), "t1", "u1", "")
        .await
        .unwrap_err();
    assert_eq!(err, "role is required");
}

#[tokio::test]
async fn change_member_role_validates_team_id_first() {
    let err = change_member_role(&cfg(), "", "u1", "admin")
        .await
        .unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn change_member_role_validates_user_id_before_role() {
    let err = change_member_role(&cfg(), "t1", "", "admin")
        .await
        .unwrap_err();
    assert_eq!(err, "userId is required");
}

#[tokio::test]
async fn list_invites_rejects_empty_team_id() {
    let err = list_invites(&cfg(), "").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn revoke_invite_rejects_empty_team_id() {
    let err = revoke_invite(&cfg(), "", "inv1").await.unwrap_err();
    assert_eq!(err, "teamId is required");
}

#[tokio::test]
async fn revoke_invite_rejects_empty_invite_id() {
    let err = revoke_invite(&cfg(), "t1", "").await.unwrap_err();
    assert_eq!(err, "inviteId is required");
}
