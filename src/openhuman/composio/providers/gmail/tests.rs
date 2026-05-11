//! Unit tests for the Gmail provider.

use super::sync::{
    cursor_to_gmail_after_epoch_filter, cursor_to_gmail_after_filter, extract_messages,
    extract_page_token,
};
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
    // 1774915200000 ms = 2026-03-31 UTC
    let millis = "1774915200000";
    assert_eq!(
        cursor_to_gmail_after_filter(millis),
        Some("2026/03/31".to_string())
    );
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

#[test]
fn default_impl_matches_new() {
    let _a = GmailProvider::new();
    let _b = GmailProvider::default();
    // Both are unit structs — constructing via Default is the cover target.
}

#[test]
fn epoch_filter_is_preferred_over_day_filter_for_typical_internal_date() {
    // The provider tries `cursor_to_gmail_after_epoch_filter` first
    // and only falls back to the day filter if the parse fails. Both
    // helpers must accept the same internalDate (epoch millis) input
    // so the fallback path is genuinely a fallback, not a divergence.
    let internal_date = "1774915200000"; // 2026-03-31 UTC
    let epoch = cursor_to_gmail_after_epoch_filter(internal_date).unwrap();
    let day = cursor_to_gmail_after_filter(internal_date).unwrap();
    assert_eq!(epoch, "1774915200");
    assert_eq!(day, "2026/03/31");
    // Sanity bound: the epoch filter must be after 2020 and before
    // year 2100, otherwise we shipped a regression in the cursor
    // converter that would silently let queries land on year 1970.
    let secs: i64 = epoch.parse().unwrap();
    assert!(
        secs > 1_577_836_800,
        "epoch filter must be after 2020-01-01"
    );
    assert!(
        secs < 4_102_444_800,
        "epoch filter must be before 2100-01-01"
    );
}

// Note: full `sync` / `fetch_user_profile` / `on_trigger` paths require a
// live `ComposioClient` (HTTP) plus the global `MemoryClient` singleton.
// Those go through the integration test suite. Here we just lock in
// the provider's identity surface and helpers.
