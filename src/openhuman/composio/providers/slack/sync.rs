//! Helpers for the Composio-backed Slack provider.
//!
//! Split out from `provider.rs` so the response-shape parsing code can
//! evolve independently of the sync orchestration — Composio (and Slack
//! beneath it) periodically widens response envelopes, and keeping the
//! JSON-pointer walks in one file makes adding new paths cheap.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use super::users::SlackUsers;
use crate::openhuman::memory::slack_ingestion::types::{SlackChannel, SlackMessage};

/// Walk the Composio response envelope and pull out the channel array
/// from a `SLACK_LIST_CONVERSATIONS` call. Composio often wraps the raw
/// upstream shape one or two levels deeper, so we try multiple pointers.
pub(crate) fn extract_channels(data: &Value) -> Vec<SlackChannel> {
    let candidates = [
        data.pointer("/data/channels"),
        data.pointer("/channels"),
        data.pointer("/data/data/channels"),
        data.pointer("/data/conversations"),
        data.pointer("/conversations"),
    ];
    let arr = candidates
        .into_iter()
        .flatten()
        .find_map(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    arr.into_iter().filter_map(parse_channel).collect()
}

fn parse_channel(raw: Value) -> Option<SlackChannel> {
    let id = raw
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() {
        return None;
    }
    let name = raw
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();
    let is_private = raw
        .get("is_private")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some(SlackChannel {
        id,
        name,
        is_private,
    })
}

/// Walk the Composio response envelope and pull out the `messages` array
/// from a `SLACK_FETCH_CONVERSATION_HISTORY` call.
///
/// `users` resolves Slack user ids both as the message author and inline
/// `<@…>` mentions in the text. Pass [`SlackUsers::empty`] to skip
/// resolution — raw ids will pass through unchanged.
pub(crate) fn extract_messages(
    data: &Value,
    channel_id: &str,
    users: &SlackUsers,
) -> Vec<SlackMessage> {
    let candidates = [
        data.pointer("/data/messages"),
        data.pointer("/messages"),
        data.pointer("/data/data/messages"),
    ];
    let arr = candidates
        .into_iter()
        .flatten()
        .find_map(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    arr.into_iter()
        .filter_map(|raw| parse_message(channel_id, raw, users))
        .collect()
}

fn parse_message(channel_id: &str, raw: Value, users: &SlackUsers) -> Option<SlackMessage> {
    let ts_raw = raw.get("ts").and_then(|t| t.as_str())?.to_string();
    let timestamp = parse_ts(&ts_raw)?;
    let raw_text = raw
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    if raw_text.trim().is_empty() {
        return None;
    }
    // Replace `<@Uxxx>` mentions with `@<resolved>` so the canonical
    // markdown reads naturally.
    let text = users.replace_mentions(&raw_text);
    let author_id = raw
        .get("user")
        .or_else(|| raw.get("bot_id"))
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let author = users.resolve(&author_id);
    let thread_ts = raw
        .get("thread_ts")
        .and_then(|t| t.as_str())
        .map(String::from);
    Some(SlackMessage {
        channel_id: channel_id.to_string(),
        author,
        text,
        timestamp,
        ts_raw,
        thread_ts,
    })
}

/// Slack's `ts` is a decimal string `"<unix_seconds>.<micro>"`. The
/// integer part is what we care about for bucketing.
fn parse_ts(ts_raw: &str) -> Option<DateTime<Utc>> {
    let seconds_str = ts_raw.split('.').next()?;
    let secs: i64 = seconds_str.parse().ok()?;
    Utc.timestamp_opt(secs, 0).single()
}

/// Walk a `SLACK_SEARCH_MESSAGES` response envelope and pull out every
/// matching message. Unlike `extract_messages` (which is per-channel),
/// search results carry a `channel.id` field on each match — so the
/// returned `SlackMessage`s span every channel that matched the query.
pub(crate) fn extract_search_messages(data: &Value, users: &SlackUsers) -> Vec<SlackMessage> {
    let candidates = [
        data.pointer("/data/messages/matches"),
        data.pointer("/messages/matches"),
        data.pointer("/data/data/messages/matches"),
    ];
    let arr = candidates
        .into_iter()
        .flatten()
        .find_map(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    arr.into_iter()
        .filter_map(|raw| parse_search_match(raw, users))
        .collect()
}

/// Total page count from the search response. Slack's `search.messages`
/// uses page-number pagination (1-indexed) under
/// `messages.paging.pages`.
pub(crate) fn extract_search_total_pages(data: &Value) -> u32 {
    let candidates = [
        data.pointer("/data/messages/paging/pages"),
        data.pointer("/messages/paging/pages"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|v| v.as_u64())
        .unwrap_or(1) as u32
}

fn parse_search_match(raw: Value, users: &SlackUsers) -> Option<SlackMessage> {
    let ts_raw = raw.get("ts").and_then(|t| t.as_str())?.to_string();
    let timestamp = parse_ts(&ts_raw)?;
    let raw_text = raw
        .get("text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();
    if raw_text.trim().is_empty() {
        return None;
    }
    let text = users.replace_mentions(&raw_text);
    let author_id = raw
        .get("user")
        .or_else(|| raw.get("bot_id"))
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let author = users.resolve(&author_id);
    let channel_id = raw
        .pointer("/channel/id")
        .and_then(|c| c.as_str())?
        .to_string();
    let thread_ts = raw
        .get("thread_ts")
        .and_then(|t| t.as_str())
        .map(String::from);
    Some(SlackMessage {
        channel_id,
        author,
        text,
        timestamp,
        ts_raw,
        thread_ts,
    })
}

/// Extract a pagination `next_cursor` from a `SLACK_LIST_CONVERSATIONS`
/// or `SLACK_FETCH_CONVERSATION_HISTORY` response.
pub(crate) fn extract_next_cursor(data: &Value) -> Option<String> {
    let candidates = [
        data.pointer("/data/response_metadata/next_cursor"),
        data.pointer("/response_metadata/next_cursor"),
        data.pointer("/data/next_cursor"),
        data.pointer("/next_cursor"),
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

/// Per-channel cursor map encoded into `SyncState.cursor`. We use
/// `BTreeMap` so serialization is deterministic (makes log diffs
/// readable and tests stable).
///
/// Value is unix-seconds of the **end of the latest flushed bucket** for
/// that channel. Fetches for that channel use `oldest = value` so we
/// skip already-ingested ranges.
pub type ChannelCursors = BTreeMap<String, i64>;

/// Deserialize the per-channel cursor map out of `SyncState.cursor`.
/// Returns an empty map on any parse failure — a "broken" cursor should
/// degrade to "start from the backfill window" rather than bail out.
pub(crate) fn decode_cursors(raw: Option<&str>) -> ChannelCursors {
    let Some(raw) = raw else {
        return ChannelCursors::new();
    };
    match serde_json::from_str::<ChannelCursors>(raw) {
        Ok(map) => map,
        Err(err) => {
            tracing::warn!(
                error = %err,
                "[composio:slack] cursor parse failed, resetting per-channel cursors"
            );
            ChannelCursors::new()
        }
    }
}

pub(crate) fn encode_cursors(map: &ChannelCursors) -> String {
    serde_json::to_string(map).unwrap_or_else(|_| "{}".to_string())
}

pub(crate) fn now_ms() -> u64 {
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
    fn extract_channels_from_data_channels() {
        let data = json!({
            "data": {
                "channels": [
                    {"id": "C1", "name": "eng", "is_private": false},
                    {"id": "G1", "name": "ops", "is_private": true},
                    {"id": "", "name": "empty-id-drops"},
                ]
            }
        });
        let out = extract_channels(&data);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "C1");
        assert!(!out[0].is_private);
        assert_eq!(out[1].id, "G1");
        assert!(out[1].is_private);
    }

    #[test]
    fn extract_channels_honors_nested_envelope() {
        let data = json!({
            "data": { "data": { "channels": [{"id": "C1", "name": "eng"}] } }
        });
        let out = extract_channels(&data);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "C1");
    }

    #[test]
    fn extract_messages_parses_fields() {
        let data = json!({
            "data": {
                "messages": [
                    {"ts": "1714003200.000100", "user": "U1", "text": "hi"},
                    {"ts": "1714003300.000200", "user": "U2", "text": "world"},
                    {"ts": "1714003400.000300", "user": "U3", "text": "  "} // dropped (blank)
                ]
            }
        });
        let users = SlackUsers::empty();
        let out = extract_messages(&data, "C1", &users);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].channel_id, "C1");
        // Empty user-cache passes raw id through unchanged.
        assert_eq!(out[0].author, "U1");
        assert_eq!(out[0].text, "hi");
        assert_eq!(out[0].timestamp.timestamp(), 1_714_003_200);
        assert_eq!(out[0].ts_raw, "1714003200.000100");
    }

    #[test]
    fn extract_messages_resolves_authors_and_mentions() {
        let data = json!({
            "data": {
                "messages": [
                    {"ts": "1714003200.0", "user": "U1", "text": "ping <@U2> about the migration"}
                ]
            }
        });
        let mut m = std::collections::HashMap::new();
        m.insert("U1".into(), "alice".into());
        m.insert("U2".into(), "bob".into());
        let users = SlackUsers::from_map(m);
        let out = extract_messages(&data, "C1", &users);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].author, "alice");
        assert_eq!(out[0].text, "ping @bob about the migration");
    }

    #[test]
    fn extract_messages_skips_missing_ts() {
        let data = json!({"messages": [{"user": "U1", "text": "orphan"}]});
        let users = SlackUsers::empty();
        let out = extract_messages(&data, "C1", &users);
        assert!(out.is_empty());
    }

    #[test]
    fn extract_next_cursor_finds_response_metadata_path() {
        let data = json!({
            "data": {
                "response_metadata": { "next_cursor": "dXNlcjpVMDY..." }
            }
        });
        assert_eq!(
            extract_next_cursor(&data),
            Some("dXNlcjpVMDY...".to_string())
        );
    }

    #[test]
    fn extract_next_cursor_none_when_blank() {
        let data = json!({"data": {"response_metadata": {"next_cursor": "  "}}});
        assert!(extract_next_cursor(&data).is_none());
    }

    #[test]
    fn encode_decode_roundtrip() {
        let mut map = ChannelCursors::new();
        map.insert("C1".into(), 1_714_003_200);
        map.insert("C2".into(), 1_714_010_000);
        let encoded = encode_cursors(&map);
        let decoded = decode_cursors(Some(&encoded));
        assert_eq!(decoded, map);
    }

    #[test]
    fn decode_empty_cursor_returns_empty_map() {
        assert!(decode_cursors(None).is_empty());
        assert!(decode_cursors(Some("")).is_empty());
        assert!(decode_cursors(Some("not json")).is_empty());
    }

    #[test]
    fn parse_ts_accepts_slack_decimal_format() {
        let dt = parse_ts("1714003200.000100").unwrap();
        assert_eq!(dt.timestamp(), 1_714_003_200);
    }

    #[test]
    fn parse_ts_rejects_garbage() {
        assert!(parse_ts("").is_none());
        assert!(parse_ts("not.a.number").is_none());
    }
}
