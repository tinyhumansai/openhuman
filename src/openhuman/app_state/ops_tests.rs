use super::*;
use serde_json::json;

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
