//! Message / user / channel extraction from raw Slack IDB records.
//!
//! Slack's Redux-persist snapshots nest arbitrarily — message arrays live
//! inside `messages[channelId]` objects inside a `state` record inside a
//! store record. Rather than pin the walk to a specific schema (which
//! moves across Slack versions), we recurse depth-first and match shapes.
//!
//! Matchers:
//!   * **Message** — an object with a Slack-shaped `ts` (`<10d>.<1-8d>`),
//!     a non-empty `text`, and a `user`/`bot_id`/`username`. Records with
//!     `type == "message"` are preferred when available.
//!   * **User** — any record with an `id` starting with `U`/`W` and a
//!     non-empty `profile.real_name` / `profile.display_name` / `real_name`
//!     / `name`.
//!   * **Channel** — any record with an `id` starting with `C` / `G` / `D`
//!     and a non-empty `name_normalized` / `name`.
//!   * **Workspace name** — the first record with an `id` starting with
//!     `T` that carries a non-empty `name`.
//!
//! Redux-persist sometimes stores serialised state as JSON-encoded strings;
//! if we hit a string that looks JSON-ish we parse it and recurse. Depth
//! is capped at 40 so pathological graphs can't loop.

use std::collections::HashMap;

use serde_json::Value;

use super::{idb::IdbDump, looks_like_slack_ts};

#[derive(Debug, Default)]
pub struct ExtractedMessage {
    pub channel: String,
    pub user: String,
    pub text: String,
    pub ts: String,
}

/// Main entry: walks every record in the dump and returns
/// `(messages, user_id → display_name, channel_id → name, workspace_name)`.
pub fn harvest(
    dump: &IdbDump,
) -> (
    Vec<ExtractedMessage>,
    HashMap<String, String>,
    HashMap<String, String>,
    Option<String>,
) {
    let mut messages: Vec<ExtractedMessage> = Vec::new();
    let mut users: HashMap<String, String> = HashMap::new();
    let mut channels: HashMap<String, String> = HashMap::new();
    let mut workspace: Option<String> = None;

    for db in &dump.dbs {
        for store in &db.stores {
            for rec in &store.records {
                // Context from parent key: many Slack message arrays live
                // under `messages["C12345"] = [...]`, so we seed the
                // recursion with the store's enclosing channel hint when
                // available.
                walk(
                    rec,
                    None,
                    &mut messages,
                    &mut users,
                    &mut channels,
                    &mut workspace,
                    0,
                );
            }
        }
    }
    (messages, users, channels, workspace)
}

fn walk(
    v: &Value,
    channel_hint: Option<&str>,
    messages: &mut Vec<ExtractedMessage>,
    users: &mut HashMap<String, String>,
    channels: &mut HashMap<String, String>,
    workspace: &mut Option<String>,
    depth: u32,
) {
    if depth > 40 {
        return;
    }
    match v {
        Value::Object(map) => {
            // 1) Message-shape check.
            if let Some(ts) = map.get("ts").and_then(|v| v.as_str()) {
                if looks_like_slack_ts(ts) {
                    let text = map
                        .get("text")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .unwrap_or_default();
                    let user = map
                        .get("user")
                        .and_then(|v| v.as_str())
                        .or_else(|| map.get("bot_id").and_then(|v| v.as_str()))
                        .or_else(|| map.get("username").and_then(|v| v.as_str()))
                        .unwrap_or("")
                        .to_string();
                    let channel = map
                        .get("channel")
                        .and_then(|v| v.as_str())
                        .or_else(|| map.get("channel_id").and_then(|v| v.as_str()))
                        .map(str::to_string)
                        .or_else(|| channel_hint.map(str::to_string))
                        .unwrap_or_default();
                    let is_message = map
                        .get("type")
                        .and_then(|v| v.as_str())
                        .map(|s| s == "message")
                        .unwrap_or(false)
                        || (!text.trim().is_empty() && !user.is_empty());
                    if is_message && !text.trim().is_empty() {
                        messages.push(ExtractedMessage {
                            channel,
                            user: user.clone(),
                            text,
                            ts: ts.to_string(),
                        });
                        // Inline user profile scrape.
                        if let Some(prof) = map.get("user_profile").and_then(|v| v.as_object()) {
                            if !user.is_empty() {
                                if let Some(name) = prof
                                    .get("real_name")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| prof.get("display_name").and_then(|v| v.as_str()))
                                    .filter(|s| !s.is_empty())
                                {
                                    users
                                        .entry(user.clone())
                                        .or_insert_with(|| name.to_string());
                                }
                            }
                        }
                    }
                }
            }

            // 2) User / channel / team shape checks via leading id char.
            if let Some(id) = map.get("id").and_then(|v| v.as_str()) {
                let first = id.chars().next().unwrap_or('\0');
                match first {
                    'U' | 'W' => {
                        let name = map
                            .get("profile")
                            .and_then(|p| p.get("real_name"))
                            .and_then(|v| v.as_str())
                            .or_else(|| {
                                map.get("profile")
                                    .and_then(|p| p.get("display_name"))
                                    .and_then(|v| v.as_str())
                            })
                            .or_else(|| map.get("real_name").and_then(|v| v.as_str()))
                            .or_else(|| map.get("name").and_then(|v| v.as_str()))
                            .filter(|s| !s.is_empty());
                        if let Some(n) = name {
                            users.entry(id.to_string()).or_insert_with(|| n.to_string());
                        }
                    }
                    'C' | 'G' | 'D' => {
                        let name = map
                            .get("name_normalized")
                            .and_then(|v| v.as_str())
                            .or_else(|| map.get("name").and_then(|v| v.as_str()))
                            .filter(|s| !s.is_empty());
                        if let Some(n) = name {
                            channels
                                .entry(id.to_string())
                                .or_insert_with(|| n.to_string());
                        }
                    }
                    'T' => {
                        if workspace.is_none() {
                            if let Some(n) = map
                                .get("name")
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                            {
                                *workspace = Some(n.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }

            // 3) Recurse into children. If the current key looks like a
            // channel id (C…/G…/D…), pass it down as a hint so messages
            // nested under it without a `channel` field still get grouped
            // correctly.
            for (k, vv) in map.iter() {
                let next_hint = if is_channel_id(k) {
                    Some(k.as_str())
                } else {
                    channel_hint
                };
                walk(
                    vv,
                    next_hint,
                    messages,
                    users,
                    channels,
                    workspace,
                    depth + 1,
                );
            }
        }
        Value::Array(arr) => {
            for vv in arr.iter() {
                walk(
                    vv,
                    channel_hint,
                    messages,
                    users,
                    channels,
                    workspace,
                    depth + 1,
                );
            }
        }
        Value::String(s) => {
            // Redux-persist default: values are JSON-encoded strings. If
            // this string is plausibly JSON, parse and recurse.
            if s.len() > 20
                && (s.starts_with('{') || s.starts_with('['))
                && (s.ends_with('}') || s.ends_with(']'))
            {
                if let Ok(inner) = serde_json::from_str::<Value>(s) {
                    walk(
                        &inner,
                        channel_hint,
                        messages,
                        users,
                        channels,
                        workspace,
                        depth + 1,
                    );
                }
            }
        }
        _ => {}
    }
}

fn is_channel_id(s: &str) -> bool {
    let mut chars = s.chars();
    let first = match chars.next() {
        Some(c) => c,
        None => return false,
    };
    if !matches!(first, 'C' | 'G' | 'D') {
        return false;
    }
    // Slack ids are uppercase alphanumeric, typically 9-11 chars.
    s.len() >= 9 && s.len() <= 12 && s.chars().all(|c| c.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_dump() -> IdbDump {
        IdbDump::default()
    }

    #[test]
    fn extracts_message_shape() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "ReduxPersistIDB:T123_U456".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "state".into(),
                count: 1,
                records: vec![json!({
                    "messages": {
                        "C0000000A1": [
                            {
                                "type": "message",
                                "ts": "1712345678.000200",
                                "user": "U111",
                                "text": "hello",
                            }
                        ]
                    }
                })],
                error: None,
            }],
            error: None,
        });
        let (msgs, _users, _chans, _ws) = harvest(&dump);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].channel, "C0000000A1");
        assert_eq!(msgs[0].user, "U111");
        assert_eq!(msgs[0].text, "hello");
        assert_eq!(msgs[0].ts, "1712345678.000200");
    }

    #[test]
    fn picks_up_user_and_channel_directories() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "ReduxPersistIDB:T123".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "state".into(),
                count: 1,
                records: vec![json!({
                    "users": [
                        { "id": "U111", "profile": { "real_name": "Ada Lovelace" }}
                    ],
                    "channels": [
                        { "id": "C0000000A1", "name": "general" }
                    ],
                    "team": { "id": "T123", "name": "Acme Inc." }
                })],
                error: None,
            }],
            error: None,
        });
        let (_msgs, users, chans, ws) = harvest(&dump);
        assert_eq!(users.get("U111").map(String::as_str), Some("Ada Lovelace"));
        assert_eq!(chans.get("C0000000A1").map(String::as_str), Some("general"));
        assert_eq!(ws.as_deref(), Some("Acme Inc."));
    }

    #[test]
    fn recurses_into_json_encoded_strings() {
        let mut dump = empty_dump();
        let inner = json!({
            "ts": "1712345678.000200",
            "text": "nested",
            "user": "U111",
            "channel": "C0000000A1",
            "type": "message",
        })
        .to_string();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "ReduxPersistIDB:T123".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "state".into(),
                count: 1,
                records: vec![json!({ "slice": inner })],
                error: None,
            }],
            error: None,
        });
        let (msgs, _, _, _) = harvest(&dump);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "nested");
    }
}
