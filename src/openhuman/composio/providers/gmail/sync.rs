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
/// `after:YYYY/MM/DD` filter component. Returns `None` if the cursor
/// cannot be parsed.
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

pub(crate) fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
