//! Unit tests for the Gmail provider.

use super::sync::{cursor_to_gmail_after_filter, extract_messages, extract_page_token};
use super::GmailProvider;
use crate::openhuman::composio::providers::ComposioProvider;
use serde_json::json;

#[test]
fn extract_messages_finds_data_messages() {
    let v = json!({
        "data": { "messages": [{"id": "m1"}, {"id": "m2"}] },
        "successful": true,
    });
    assert_eq!(extract_messages(&v).len(), 2);
}

#[test]
fn extract_messages_finds_top_level_messages() {
    let v = json!({ "messages": [{"id": "m1"}] });
    assert_eq!(extract_messages(&v).len(), 1);
}

#[test]
fn extract_messages_returns_empty_when_missing() {
    let v = json!({ "data": { "other": [] } });
    assert_eq!(extract_messages(&v).len(), 0);
}

#[test]
fn extract_page_token_finds_nested() {
    let v = json!({ "data": { "nextPageToken": "tok123" } });
    assert_eq!(extract_page_token(&v), Some("tok123".to_string()));
}

#[test]
fn extract_page_token_none_when_missing() {
    let v = json!({ "data": {} });
    assert_eq!(extract_page_token(&v), None);
}

#[test]
fn cursor_to_filter_from_epoch_millis() {
    // 2026-04-01 00:00:00 UTC in millis
    let millis = "1774915200000";
    let filter = cursor_to_gmail_after_filter(millis);
    assert!(filter.is_some());
    // Should produce a YYYY/MM/DD date.
    let f = filter.unwrap();
    assert!(f.contains('/'), "Expected date with slashes, got {f}");
}

#[test]
fn cursor_to_filter_from_iso_date() {
    assert_eq!(
        cursor_to_gmail_after_filter("2026-03-15"),
        Some("2026/03/15".to_string())
    );
}

#[test]
fn cursor_to_filter_from_rfc3339() {
    let f = cursor_to_gmail_after_filter("2026-03-15T12:00:00Z");
    assert_eq!(f, Some("2026/03/15".to_string()));
}

#[test]
fn cursor_to_filter_returns_none_for_garbage() {
    assert_eq!(cursor_to_gmail_after_filter("not-a-date"), None);
}

#[test]
fn provider_metadata_is_stable() {
    let p = GmailProvider::new();
    assert_eq!(p.toolkit_slug(), "gmail");
    assert_eq!(p.sync_interval_secs(), Some(15 * 60));
}
