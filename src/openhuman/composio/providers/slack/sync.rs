//! Helpers for the Composio-backed Slack provider.
//!
//! This module contains thin enrichers that take a post-processed slim
//! envelope (produced by [`super::post_process`]) and turn it into
//! [`SlackMessage`] / [`SlackChannel`] values with user-id resolution and
//! channel-context injection applied.
//!
//! Response-shape walking (nested envelopes, empty-field filtering) lives in
//! `post_process.rs`; this module assumes the slim shape is already in place.

use std::collections::{BTreeMap, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

use super::types::{SlackChannel, SlackMessage};
use super::users::SlackUsers;

/// Enrich the top-level `channels[]` array in a post-processed
/// `SLACK_LIST_CONVERSATIONS` response into [`SlackChannel`] values.
///
/// The post-processor has already stripped unknown channels and normalised
/// to `{ id, name, is_private }` — this function just deserialises them.
pub(crate) fn extract_channels(data: &Value) -> Vec<SlackChannel> {
    let arr = data
        .get("channels")
        .and_then(|v| v.as_array())
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

/// Enrich the top-level `messages[]` array in a post-processed
/// `SLACK_FETCH_CONVERSATION_HISTORY` response into [`SlackMessage`]s.
///
/// `channel` provides the channel id, name, and privacy flag (not present
/// in the response body — only in the request). `users` resolves author ids
/// and rewrites `<@…>` mentions in message text.
pub(crate) fn extract_messages(
    data: &Value,
    channel: &SlackChannel,
    users: &SlackUsers,
) -> Vec<SlackMessage> {
    let arr = data
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    arr.into_iter()
        .filter_map(|raw| parse_message(raw, channel, users))
        .collect()
}

fn parse_message(raw: Value, channel: &SlackChannel, users: &SlackUsers) -> Option<SlackMessage> {
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
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let author = users.resolve(&author_id);
    let thread_ts = raw
        .get("thread_ts")
        .and_then(|t| t.as_str())
        .map(String::from);
    let permalink = raw
        .get("permalink")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Some(SlackMessage {
        channel_id: channel.id.clone(),
        channel_name: channel.name.clone(),
        is_private: channel.is_private,
        author,
        author_id,
        text,
        timestamp,
        ts_raw,
        thread_ts,
        permalink,
    })
}

/// Enrich the top-level `messages[]` array in a post-processed
/// `SLACK_SEARCH_MESSAGES` response into [`SlackMessage`]s.
///
/// `channel_map` provides channel names and privacy flags keyed by id.
/// When a match's `channel_id` is absent from the map, channel name and
/// privacy default to empty/false — the message is still ingested but
/// the label will be less informative.
pub(crate) fn extract_search_messages(
    data: &Value,
    channel_map: &HashMap<String, SlackChannel>,
    users: &SlackUsers,
) -> Vec<SlackMessage> {
    let arr = data
        .get("messages")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    arr.into_iter()
        .filter_map(|raw| parse_search_match(raw, channel_map, users))
        .collect()
}

fn parse_search_match(
    raw: Value,
    channel_map: &HashMap<String, SlackChannel>,
    users: &SlackUsers,
) -> Option<SlackMessage> {
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
        .and_then(|u| u.as_str())
        .unwrap_or("")
        .to_string();
    let author = users.resolve(&author_id);
    let channel_id = raw
        .get("channel_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let (channel_name, is_private) = channel_map
        .get(&channel_id)
        .map(|c| (c.name.clone(), c.is_private))
        .unwrap_or_else(|| (String::new(), false));
    let thread_ts = raw
        .get("thread_ts")
        .and_then(|t| t.as_str())
        .map(String::from);
    let permalink = raw
        .get("permalink")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Some(SlackMessage {
        channel_id,
        channel_name,
        is_private,
        author,
        author_id,
        text,
        timestamp,
        ts_raw,
        thread_ts,
        permalink,
    })
}

/// Slack's `ts` is a decimal string `"<unix_seconds>.<micro>"`. The
/// integer part is what we care about for timestamp purposes.
pub(crate) fn parse_ts(ts_raw: &str) -> Option<DateTime<Utc>> {
    let seconds_str = ts_raw.split('.').next()?;
    let secs: i64 = seconds_str.parse().ok()?;
    Utc.timestamp_opt(secs, 0).single()
}

/// Extract the total page count from a post-processed
/// `SLACK_SEARCH_MESSAGES` response. Defaults to 1 when absent.
pub(crate) fn extract_search_total_pages(data: &Value) -> u32 {
    data.get("pages").and_then(|v| v.as_u64()).unwrap_or(1) as u32
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
/// Value is unix-seconds of the latest successfully-ingested message for
/// that channel. Fetches for that channel use `oldest = value` so we
/// skip already-ingested ranges.
pub type ChannelCursors = BTreeMap<String, i64>;

/// Deserialize the per-channel cursor map out of `SyncState.cursor`.
/// Returns an empty map on any parse failure — a broken cursor should
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
    fn extract_channels_from_post_processed_shape() {
        let data = json!({
            "channels": [
                {"id": "C1", "name": "eng", "is_private": false},
                {"id": "G1", "name": "ops", "is_private": true},
                {"id": "", "name": "empty-id-drops"},
            ]
        });
        let out = extract_channels(&data);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "C1");
        assert!(!out[0].is_private);
        assert_eq!(out[1].id, "G1");
        assert!(out[1].is_private);
    }

    #[test]
    fn extract_messages_parses_post_processed_shape() {
        let data = json!({
            "messages": [
                {"ts": "1714003200.000100", "user": "U1", "text": "hi"},
                {"ts": "1714003300.000200", "user": "U2", "text": "world"},
                {"ts": "1714003400.000300", "user": "U3", "text": "  "} // dropped (blank)
            ]
        });
        let channel = SlackChannel {
            id: "C1".into(),
            name: "eng".into(),
            is_private: false,
        };
        let users = SlackUsers::empty();
        let out = extract_messages(&data, &channel, &users);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].channel_id, "C1");
        assert_eq!(out[0].channel_name, "eng");
        assert!(!out[0].is_private);
        assert_eq!(out[0].author, "U1");
        assert_eq!(out[0].author_id, "U1");
        assert_eq!(out[0].text, "hi");
        assert_eq!(out[0].timestamp.timestamp(), 1_714_003_200);
    }

    #[test]
    fn extract_messages_resolves_authors_and_mentions() {
        let data = json!({
            "messages": [
                {"ts": "1714003200.0", "user": "U1", "text": "ping <@U2> about the migration"}
            ]
        });
        let channel = SlackChannel {
            id: "C1".into(),
            name: "eng".into(),
            is_private: false,
        };
        let mut m = HashMap::new();
        m.insert("U1".into(), "alice".into());
        m.insert("U2".into(), "bob".into());
        let users = SlackUsers::from_map(m);
        let out = extract_messages(&data, &channel, &users);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].author, "alice");
        assert_eq!(out[0].author_id, "U1");
        assert_eq!(out[0].text, "ping @bob about the migration");
    }

    #[test]
    fn extract_search_messages_enriches_from_channel_map() {
        let data = json!({
            "messages": [
                {"ts": "1714003200.0", "user": "U1", "text": "hello", "channel_id": "C1"},
                {"ts": "1714003300.0", "user": "U2", "text": "world", "channel_id": "C2"},
            ]
        });
        let mut channel_map = HashMap::new();
        channel_map.insert(
            "C1".to_string(),
            SlackChannel {
                id: "C1".into(),
                name: "eng".into(),
                is_private: false,
            },
        );
        // C2 not in map — should still work with empty channel_name
        let users = SlackUsers::empty();
        let out = extract_search_messages(&data, &channel_map, &users);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].channel_name, "eng");
        assert_eq!(out[1].channel_name, "");
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
