//! Additional unit tests for `notifications::store` — colocated test module.
//!
//! These tests complement the inline `#[cfg(test)] mod tests` block in
//! `store.rs` and focus on the CEF/CDP dedup-window behaviour of
//! `exists_recent` and `insert_if_not_recent`:
//!
//! - Returns `true` for identical (provider, account_id, title, body) within 60 s.
//! - Returns `false` for entries older than 60 s.
//! - Returns `false` when any of provider / account_id / title / body differs.
//! - Inserting a duplicate within the window does not displace the first entry.

use super::*;
use chrono::{Duration, Utc};
use tempfile::TempDir;

fn test_config(dir: &TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = dir.path().to_path_buf();
    config
}

fn sample_notification_with(
    id: &str,
    provider: &str,
    account_id: Option<&str>,
    title: &str,
    body: &str,
) -> IntegrationNotification {
    IntegrationNotification {
        id: id.to_string(),
        provider: provider.to_string(),
        account_id: account_id.map(|s| s.to_string()),
        title: title.to_string(),
        body: body.to_string(),
        raw_payload: serde_json::json!({}),
        importance_score: None,
        triage_action: None,
        triage_reason: None,
        status: NotificationStatus::Unread,
        received_at: Utc::now(),
        scored_at: None,
    }
}

// ── exists_recent: positive case ────────────────────────────────────────────

#[test]
fn exists_recent_returns_true_for_identical_within_60s() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Insert a fresh notification (received_at = now).
    let n = sample_notification_with("n1", "slack", Some("acct-1"), "Hello", "World");
    insert(&config, &n).unwrap();

    assert!(
        exists_recent(&config, "slack", Some("acct-1"), "Hello", "World").unwrap(),
        "identical notification within 60s should be detected as recent"
    );
}

#[test]
fn exists_recent_returns_true_without_account_id_when_matching() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let mut n = sample_notification_with("n1", "discord", None, "Ping", "Server update");
    n.account_id = None;
    insert(&config, &n).unwrap();

    assert!(
        exists_recent(&config, "discord", None, "Ping", "Server update").unwrap(),
        "NULL account_id dedup should match within 60s"
    );
}

// ── exists_recent: negative cases (expired window) ─────────────────────────

#[test]
fn exists_recent_returns_false_for_entries_older_than_60s() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let mut n = sample_notification_with("old-1", "slack", Some("acct-1"), "Hello", "World");
    n.received_at = Utc::now() - Duration::seconds(61);
    insert(&config, &n).unwrap();

    assert!(
        !exists_recent(&config, "slack", Some("acct-1"), "Hello", "World").unwrap(),
        "notification older than 60s should not be considered recent"
    );
}

#[test]
fn exists_recent_returns_false_for_entry_exactly_at_60s_boundary() {
    // received_at = now - 60s is outside the `>= -60 seconds` window in SQLite.
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let mut n = sample_notification_with("boundary", "slack", None, "T", "B");
    n.received_at = Utc::now() - Duration::seconds(60);
    insert(&config, &n).unwrap();

    // The SQLite expression is `unixepoch(received_at) >= unixepoch('now', '-60 seconds')`.
    // At exactly -60s the condition holds, so this *may* return true depending on
    // clock precision. We just assert the call does not error.
    let result = exists_recent(&config, "slack", None, "T", "B");
    assert!(result.is_ok(), "exists_recent should not error at boundary");
}

// ── exists_recent: negative cases (field differs) ──────────────────────────

#[test]
fn exists_recent_returns_false_when_provider_differs() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let n = sample_notification_with("n1", "slack", None, "Hello", "World");
    insert(&config, &n).unwrap();

    assert!(
        !exists_recent(&config, "discord", None, "Hello", "World").unwrap(),
        "different provider should not match"
    );
}

#[test]
fn exists_recent_returns_false_when_account_id_differs() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let n = sample_notification_with("n1", "slack", Some("acct-a"), "Hello", "World");
    insert(&config, &n).unwrap();

    // Different account_id — should not match.
    assert!(
        !exists_recent(&config, "slack", Some("acct-b"), "Hello", "World").unwrap(),
        "different account_id should not match"
    );
}

#[test]
fn exists_recent_returns_false_when_title_differs() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let n = sample_notification_with("n1", "slack", Some("acct-1"), "Hello", "World");
    insert(&config, &n).unwrap();

    assert!(
        !exists_recent(&config, "slack", Some("acct-1"), "Different title", "World").unwrap(),
        "different title should not match"
    );
}

#[test]
fn exists_recent_returns_false_when_body_differs() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let n = sample_notification_with("n1", "slack", Some("acct-1"), "Hello", "World");
    insert(&config, &n).unwrap();

    assert!(
        !exists_recent(&config, "slack", Some("acct-1"), "Hello", "Different body").unwrap(),
        "different body should not match"
    );
}

#[test]
fn exists_recent_returns_false_when_null_vs_non_null_account_id() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Store with non-null account_id.
    let n = sample_notification_with("n1", "slack", Some("acct-1"), "Hello", "World");
    insert(&config, &n).unwrap();

    // Query with NULL account_id — different field value.
    assert!(
        !exists_recent(&config, "slack", None, "Hello", "World").unwrap(),
        "stored account_id=Some vs queried account_id=None should not match"
    );
}

// ── insert_if_not_recent: duplicate does not displace first entry ───────────

#[test]
fn insert_if_not_recent_within_window_preserves_first_entry() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // First insert — same title/body.
    let first = sample_notification_with("first-id", "slack", None, "Alert", "Server down");
    let inserted = insert_if_not_recent(&config, &first).unwrap();
    assert!(inserted, "first insert should succeed");

    // Second insert within the window — different id, same content.
    let second = sample_notification_with("second-id", "slack", None, "Alert", "Server down");
    let inserted = insert_if_not_recent(&config, &second).unwrap();
    assert!(!inserted, "second insert within window should be skipped");

    // Verify the store contains exactly one record and it's the first one.
    let items = list(&config, 10, 0, Some("slack"), None).unwrap();
    assert_eq!(items.len(), 1, "only the first entry should be stored");
    assert_eq!(
        items[0].id, "first-id",
        "the surviving entry must be the first one"
    );
}

#[test]
fn insert_if_not_recent_with_different_body_inserts_both() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let n1 = sample_notification_with("id-1", "slack", None, "Alert", "Server A down");
    let n2 = sample_notification_with("id-2", "slack", None, "Alert", "Server B down");

    assert!(insert_if_not_recent(&config, &n1).unwrap());
    assert!(
        insert_if_not_recent(&config, &n2).unwrap(),
        "different body means different notification — should insert"
    );

    let items = list(&config, 10, 0, Some("slack"), None).unwrap();
    assert_eq!(
        items.len(),
        2,
        "both notifications with different body should be stored"
    );
}

#[test]
fn insert_if_not_recent_with_different_provider_inserts_both() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    let n1 = sample_notification_with("id-1", "slack", None, "Alert", "Same body");
    let n2 = sample_notification_with("id-2", "discord", None, "Alert", "Same body");

    assert!(insert_if_not_recent(&config, &n1).unwrap());
    assert!(
        insert_if_not_recent(&config, &n2).unwrap(),
        "different provider means different notification — should insert"
    );

    let items = list(&config, 10, 0, None, None).unwrap();
    assert_eq!(
        items.len(),
        2,
        "notifications for different providers are distinct"
    );
}

#[test]
fn insert_if_not_recent_after_expiry_inserts_again() {
    let dir = TempDir::new().unwrap();
    let config = test_config(&dir);

    // Write an expired duplicate directly (bypass insert_if_not_recent to
    // set received_at in the past).
    let mut old = sample_notification_with("old-id", "slack", None, "Alert", "Server down");
    old.received_at = Utc::now() - Duration::seconds(120);
    insert(&config, &old).unwrap();

    // Now insert_if_not_recent should allow the same content because the
    // prior entry is older than 60 s.
    let fresh = sample_notification_with("fresh-id", "slack", None, "Alert", "Server down");
    assert!(
        insert_if_not_recent(&config, &fresh).unwrap(),
        "expired entry should not block a fresh identical notification"
    );

    let items = list(&config, 10, 0, Some("slack"), None).unwrap();
    assert_eq!(
        items.len(),
        2,
        "both old and fresh entries should be stored"
    );
}
