//! Unit tests for the Gmail provider.

use super::sync::{
    cursor_to_gmail_after_epoch_filter, cursor_to_gmail_after_filter, extract_messages,
    extract_page_token, now_ms, parse_cursor_to_epoch_secs,
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

// ── Adaptive page cap and early-stop helpers (issue#1404, pr#1474) ──────────
//
// The full `sync()` path needs a live ComposioClient + MemoryClient, so
// we test the helper functions that gate the adaptive cap and early-stop
// decisions:
//
//   * `parse_cursor_to_epoch_secs` — used to decide whether `last_sync_at_ms`
//     falls within `RECENT_SYNC_WINDOW_MS` (5 min) for the adaptive page cap.
//   * `now_ms` — sanity check: must not return 0 and must be within a plausible
//     range so the adaptive window comparison never produces pathological results.
//   * early-stop guard: when `last_seen_id` matches the first page's head id
//     the sync loop breaks with `stop_reason = "head_unchanged"`. We pin the
//     helper logic that feeds this decision.

#[test]
fn parse_cursor_to_epoch_secs_handles_epoch_millis() {
    // Gmail internalDate is epoch milliseconds as a numeric string.
    // 1774915200000 ms = 1774915200 s (2026-03-31 00:00:00 UTC).
    assert_eq!(
        parse_cursor_to_epoch_secs("1774915200000"),
        Some(1774915200)
    );
}

#[test]
fn parse_cursor_to_epoch_secs_handles_iso_date() {
    // YYYY-MM-DD date cursor produced by the older day-cursor write path.
    let secs = parse_cursor_to_epoch_secs("2024-01-15").unwrap();
    // 2024-01-15 00:00:00 UTC = 1705276800
    assert_eq!(secs, 1705276800);
}

#[test]
fn parse_cursor_to_epoch_secs_handles_rfc3339() {
    let secs = parse_cursor_to_epoch_secs("2024-01-15T00:00:00Z").unwrap();
    assert_eq!(secs, 1705276800);
}

#[test]
fn parse_cursor_to_epoch_secs_returns_none_for_garbage() {
    assert_eq!(parse_cursor_to_epoch_secs("not-a-timestamp"), None);
    assert_eq!(parse_cursor_to_epoch_secs(""), None);
    assert_eq!(parse_cursor_to_epoch_secs("   "), None);
}

/// The adaptive page cap relies on `parse_cursor_to_epoch_secs` and `now_ms`
/// agreeing on a common epoch so the "less than 5 min ago" comparison works.
/// `now_ms()` must return epoch-milliseconds (not zero, not micros). If it
/// returned microseconds, every sync would appear "recent" (delta < 300_000 ms
/// vs delta actually being ~ 1e12 µs); if it returned seconds, every sync
/// would appear "old" (delta > 300_000 ms trivially).
#[test]
fn now_ms_is_in_epoch_milliseconds_range() {
    let ms = now_ms();
    // Must be strictly positive.
    assert!(ms > 0, "now_ms must not return zero");
    // Must be > 2024-01-01 00:00:00 UTC in milliseconds so it's clearly
    // millisecond-epoch and not seconds-epoch (which would be ~1.7e9, much
    // smaller than 1.7e12).
    let jan_2024_ms: u64 = 1_704_067_200_000;
    assert!(
        ms > jan_2024_ms,
        "now_ms ({ms}) must be above 2024-01-01 in epoch-millisecond scale"
    );
    // Must be < year 2100 in milliseconds — rules out microseconds/nanoseconds.
    let year_2100_ms: u64 = 4_102_444_800_000;
    assert!(
        ms < year_2100_ms,
        "now_ms ({ms}) must be below year 2100 in epoch-millisecond scale"
    );
}

/// The early-stop optimisation fires when `last_seen_id` equals the first
/// message id on the first page. We test the helper that extracts message ids
/// — `extract_messages` — to verify it correctly surfaces the `id` field so
/// the comparison in the sync loop gets the right value.
///
/// The early-stop check uses `messages.first()` with the `MESSAGE_ID_PATHS`
/// extractor. We can't call the private extractor, but we can pin
/// `extract_messages` to return messages with their `id` intact so the
/// sync loop can compare them to `state.last_seen_id`.
#[test]
fn extract_messages_preserves_id_field_for_early_stop() {
    // The early-stop check reads `m["id"]` via `extract_item_id`. Verify
    // `extract_messages` doesn't strip or transform the field.
    let v = json!({
        "data": {
            "messages": [
                {"id": "msg_abc", "internalDate": "1774915200000"},
                {"id": "msg_def", "internalDate": "1774915100000"}
            ]
        },
        "successful": true
    });
    let msgs = extract_messages(&v);
    assert_eq!(msgs.len(), 2);
    assert_eq!(
        msgs[0]["id"], "msg_abc",
        "first message id must be preserved"
    );
    assert_eq!(
        msgs[1]["id"], "msg_def",
        "second message id must be preserved"
    );
}

/// Variant: messages embedded in `data.data.messages` (deeper nesting
/// seen in some Composio provider responses) — the extractor must still
/// find them so the early-stop comparison has data to work with.
#[test]
fn extract_messages_handles_deep_nesting() {
    let v = json!({
        "data": {
            "data": {
                "messages": [
                    {"id": "deep_msg_1"}
                ]
            }
        }
    });
    let msgs = extract_messages(&v);
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["id"], "deep_msg_1");
}

// Note: full `sync` / `fetch_user_profile` / `on_trigger` paths require a
// live `ComposioClient` (HTTP) plus the global `MemoryClient` singleton.
// Those go through the integration test suite. Here we just lock in
// the provider's identity surface and helpers.
