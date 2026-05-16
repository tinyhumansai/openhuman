//! Gmail sync helpers — message extraction, pagination, cursor
//! conversion, and time utilities.

use serde_json::Value;

/// Walk the Composio response envelope and pull out message objects.
pub(crate) fn extract_messages(data: &Value) -> Vec<Value> {
    let candidates = [
        data.pointer("/data/messages"),
        data.pointer("/messages"),
        data.pointer("/data/data/messages"),
        data.pointer("/data/items"),
        data.pointer("/items"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(arr) = cand.as_array() {
            return arr.clone();
        }
    }
    Vec::new()
}

/// Try to extract a pagination token from the API response.
pub(crate) fn extract_page_token(data: &Value) -> Option<String> {
    let candidates = [
        data.pointer("/data/nextPageToken"),
        data.pointer("/nextPageToken"),
        data.pointer("/data/data/nextPageToken"),
    ];
    for cand in candidates.into_iter().flatten() {
        if let Some(s) = cand.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Convert a cursor value (epoch millis or date string) into a Gmail
/// `after:YYYY/MM/DD` filter component. Day-level precision — kept as a
/// last-resort filter for non-numeric cursors and for back-compat with
/// the older day-cursor write path. New code should prefer
/// [`cursor_to_gmail_after_epoch_filter`] which produces a
/// second-precision `after:<unix>` filter and so avoids re-fetching
/// large same-day windows on every tick.
pub(crate) fn cursor_to_gmail_after_filter(cursor: &str) -> Option<String> {
    let cursor = cursor.trim();
    // Try parsing as epoch millis first (Gmail's internalDate).
    if let Ok(millis) = cursor.parse::<i64>() {
        let secs = millis / 1000;
        if let Some(dt) = chrono::DateTime::from_timestamp(secs, 0) {
            return Some(dt.format("%Y/%m/%d").to_string());
        }
    }
    // Try parsing as an ISO date/datetime.
    if let Ok(dt) = chrono::NaiveDate::parse_from_str(cursor, "%Y-%m-%d") {
        return Some(dt.format("%Y/%m/%d").to_string());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(cursor) {
        return Some(dt.format("%Y/%m/%d").to_string());
    }
    None
}

/// Convert a cursor value (epoch millis or date string) into a Gmail
/// `after:<unix_seconds>` filter component. Gmail's search syntax
/// accepts a bare unix-seconds value for `after:` / `before:`, so the
/// filter is second-precision rather than the day-level
/// `after:YYYY/MM/DD` form. Used by the incremental sync path so a
/// same-day re-tick does not re-fetch every message Gmail has filed
/// today.
///
/// Returns `None` when the cursor cannot be parsed; callers should
/// fall back to the coarse day filter to avoid sending an unbounded
/// query.
pub(crate) fn cursor_to_gmail_after_epoch_filter(cursor: &str) -> Option<String> {
    let secs = parse_cursor_to_epoch_secs(cursor)?;
    Some(secs.to_string())
}

/// Parse a cursor (epoch millis as string, `YYYY-MM-DD`, or RFC3339)
/// into unix-seconds. Shared by the epoch filter and by the adaptive
/// page-cap recency check.
pub(crate) fn parse_cursor_to_epoch_secs(cursor: &str) -> Option<i64> {
    let cursor = cursor.trim();
    if let Ok(millis) = cursor.parse::<i64>() {
        return Some(millis / 1000);
    }
    if let Ok(date) = chrono::NaiveDate::parse_from_str(cursor, "%Y-%m-%d") {
        return date.and_hms_opt(0, 0, 0).map(|dt| dt.and_utc().timestamp());
    }
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(cursor) {
        return Some(dt.timestamp());
    }
    None
}

pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_messages_from_data_messages() {
        let data = json!({"data": {"messages": [{"id": "1"}, {"id": "2"}]}});
        let msgs = extract_messages(&data);
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn extract_messages_from_top_level() {
        let data = json!({"messages": [{"id": "1"}]});
        let msgs = extract_messages(&data);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn extract_messages_from_data_items() {
        let data = json!({"data": {"items": [{"id": "a"}]}});
        let msgs = extract_messages(&data);
        assert_eq!(msgs.len(), 1);
    }

    #[test]
    fn extract_messages_empty_when_no_match() {
        let data = json!({"foo": "bar"});
        assert!(extract_messages(&data).is_empty());
    }

    #[test]
    fn extract_page_token_from_data() {
        let data = json!({"data": {"nextPageToken": "abc123"}});
        assert_eq!(extract_page_token(&data), Some("abc123".into()));
    }

    #[test]
    fn extract_page_token_from_top_level() {
        let data = json!({"nextPageToken": "tok"});
        assert_eq!(extract_page_token(&data), Some("tok".into()));
    }

    #[test]
    fn extract_page_token_none_when_empty() {
        let data = json!({"data": {"nextPageToken": "  "}});
        assert_eq!(extract_page_token(&data), None);
    }

    #[test]
    fn extract_page_token_none_when_missing() {
        let data = json!({"data": {}});
        assert_eq!(extract_page_token(&data), None);
    }

    #[test]
    fn cursor_to_filter_epoch_millis() {
        let filter = cursor_to_gmail_after_filter("1700000000000").unwrap();
        assert!(filter.contains('/'));
        assert_eq!(filter, "2023/11/14");
    }

    #[test]
    fn cursor_to_filter_iso_date() {
        let filter = cursor_to_gmail_after_filter("2024-01-15").unwrap();
        assert_eq!(filter, "2024/01/15");
    }

    #[test]
    fn cursor_to_filter_rfc3339() {
        let filter = cursor_to_gmail_after_filter("2024-06-01T12:00:00Z").unwrap();
        assert_eq!(filter, "2024/06/01");
    }

    #[test]
    fn cursor_to_filter_invalid_returns_none() {
        assert!(cursor_to_gmail_after_filter("not-a-date").is_none());
    }

    #[test]
    fn cursor_to_filter_trims_whitespace() {
        let filter = cursor_to_gmail_after_filter("  2024-01-15  ").unwrap();
        assert_eq!(filter, "2024/01/15");
    }

    #[test]
    fn now_ms_returns_nonzero() {
        assert!(now_ms() > 0);
    }

    // ── second-precision cursor ──────────────────────────────────

    #[test]
    fn epoch_filter_emits_unix_seconds_for_internal_date_millis() {
        let filter = cursor_to_gmail_after_epoch_filter("1700000000000").unwrap();
        assert_eq!(filter, "1700000000");
    }

    #[test]
    fn epoch_filter_handles_iso_date() {
        let filter = cursor_to_gmail_after_epoch_filter("2024-01-15").unwrap();
        // 2024-01-15 00:00:00 UTC == 1705276800.
        assert_eq!(filter, "1705276800");
    }

    #[test]
    fn epoch_filter_handles_rfc3339() {
        let filter = cursor_to_gmail_after_epoch_filter("2024-06-01T12:00:00Z").unwrap();
        assert_eq!(filter, "1717243200");
    }

    #[test]
    fn epoch_filter_returns_none_for_garbage() {
        assert!(cursor_to_gmail_after_epoch_filter("not-a-date").is_none());
    }

    #[test]
    fn epoch_filter_trims_whitespace() {
        let filter = cursor_to_gmail_after_epoch_filter("  2024-01-15  ").unwrap();
        assert_eq!(filter, "1705276800");
    }

    #[test]
    fn parse_cursor_round_trip_matches_epoch_filter() {
        // The adaptive page-cap relies on parse_cursor_to_epoch_secs
        // agreeing with cursor_to_gmail_after_epoch_filter — both must
        // emit the same seconds value for any given input.
        for cursor in ["1700000000000", "2024-01-15", "2024-06-01T12:00:00Z"] {
            let secs = parse_cursor_to_epoch_secs(cursor).unwrap();
            let filter = cursor_to_gmail_after_epoch_filter(cursor).unwrap();
            assert_eq!(filter, secs.to_string(), "cursor `{cursor}`");
        }
    }
}
