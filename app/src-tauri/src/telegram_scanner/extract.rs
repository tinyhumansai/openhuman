//! Message / user / chat extraction from raw Telegram Web K IDB records.
//!
//! Telegram Web K persists messages, dialogs, users, and chats into the
//! `tweb` IndexedDB. Exact schema names have moved across tweb versions,
//! so rather than pin the walk to specific (database, store) pairs we
//! recurse depth-first and match record shapes — same pattern as the
//! Slack extractor.
//!
//! Matchers:
//!   * **Message** — an object with a plausible unix-seconds `date`
//!     (10-digit int in the 2000s/current era), a non-empty `message`
//!     (or `text`) string, and either a `peerId` / `peer_id` identifier
//!     or an inherited channel/peer hint from an enclosing key.
//!   * **User** — any record with an integer `id` and at least one of
//!     `first_name`, `last_name`, `username`.
//!   * **Chat / channel** — any record with an integer `id` and a
//!     non-empty `title`. Telegram uses the same `chats` table for
//!     groups and channels; we flatten to a single (id → name) map.
//!   * **Own user / session** — the first record carrying `self: true`
//!     or `is_self: true` populates the "me" identity.
//!
//! Peer IDs in tweb can appear in two shapes:
//!   * Integer — positive for users, the app applies a prefix shift to
//!     distinguish chats vs channels internally. We treat any integer as
//!     the raw key and resolve names via the users/chats maps.
//!   * Object — `{ _: "peerUser" | "peerChat" | "peerChannel", user_id |
//!     chat_id | channel_id: <int> }` (TL schema style).
//!
//! Depth is capped at 40 so pathological graphs can't loop.

use std::collections::HashMap;

use serde_json::Value;

/// Plausibility window for unix-second `date` values — 2015-01-01 to
/// roughly year 2100. Anything outside is noise (file sizes, version
/// numbers, ids, etc.).
const DATE_MIN: i64 = 1_420_070_400;
const DATE_MAX: i64 = 4_102_444_800;

#[derive(Debug, Default, Clone)]
pub struct ExtractedMessage {
    pub peer: String,
    pub sender: String,
    pub text: String,
    pub date: i64,
}

#[derive(Debug, Default)]
pub struct Harvest {
    pub messages: Vec<ExtractedMessage>,
    pub users: HashMap<String, String>,
    pub chats: HashMap<String, String>,
    pub self_id: Option<String>,
}

/// Main entry: walks every record in the dump and returns the grouped
/// harvest.
pub fn harvest(dump: &super::idb::IdbDump) -> Harvest {
    let mut out = Harvest::default();

    for db in &dump.dbs {
        for store in &db.stores {
            for rec in &store.records {
                walk(rec, None, &mut out, 0);
            }
        }
    }
    out
}

fn walk(v: &Value, peer_hint: Option<&str>, out: &mut Harvest, depth: u32) {
    if depth > 40 {
        return;
    }
    match v {
        Value::Object(map) => {
            // 1) Message-shape check: needs (date, message|text, peer).
            if let Some(date) = map.get("date").and_then(|v| v.as_i64()) {
                if (DATE_MIN..=DATE_MAX).contains(&date) {
                    let text = map
                        .get("message")
                        .and_then(|v| v.as_str())
                        .or_else(|| map.get("text").and_then(|v| v.as_str()))
                        .map(str::to_string)
                        .unwrap_or_default();
                    if !text.trim().is_empty() {
                        let peer = extract_peer(map).or_else(|| peer_hint.map(str::to_string));
                        let sender = extract_sender(map).unwrap_or_default();
                        if let Some(peer) = peer {
                            out.messages.push(ExtractedMessage {
                                peer,
                                sender,
                                text,
                                date,
                            });
                        }
                    }
                }
            }

            // 2) User / chat directory entries (have a numeric `id`).
            if let Some(id) = map.get("id").and_then(num_to_str) {
                // User: `first_name` / `last_name` / `username` present.
                let user_name = map
                    .get("first_name")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|first| {
                        let last = map
                            .get("last_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .trim();
                        if last.is_empty() {
                            first.to_string()
                        } else {
                            format!("{first} {last}")
                        }
                    })
                    .or_else(|| {
                        map.get("username")
                            .and_then(|v| v.as_str())
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    });
                if let Some(name) = user_name {
                    out.users.entry(id.clone()).or_insert(name);

                    // Track the "self" user if the record marks itself.
                    let is_self = map.get("self").and_then(|v| v.as_bool()).unwrap_or(false)
                        || map
                            .get("is_self")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                    if is_self && out.self_id.is_none() {
                        out.self_id = Some(id.clone());
                    }
                }

                // Chat / channel: `title` present.
                if let Some(title) = map
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                {
                    out.chats
                        .entry(id.clone())
                        .or_insert_with(|| title.to_string());
                }
            }

            // 3) Recurse. If the current key looks like a peer id we pass
            //    it down as a hint so nested message arrays group correctly.
            for (k, vv) in map.iter() {
                let next_hint = if looks_like_peer_key(k) {
                    Some(k.as_str())
                } else {
                    peer_hint
                };
                walk(vv, next_hint, out, depth + 1);
            }
        }
        Value::Array(arr) => {
            for vv in arr.iter() {
                walk(vv, peer_hint, out, depth + 1);
            }
        }
        Value::String(s) => {
            // Some tweb stores persist state as JSON-encoded strings.
            // Recurse when the shape looks plausibly JSON.
            if s.len() > 20
                && (s.starts_with('{') || s.starts_with('['))
                && (s.ends_with('}') || s.ends_with(']'))
            {
                if let Ok(inner) = serde_json::from_str::<Value>(s) {
                    walk(&inner, peer_hint, out, depth + 1);
                }
            }
        }
        _ => {}
    }
}

/// Pull the peer identifier out of a message record. Handles both the
/// integer and TL-object (`{ _: "peerUser", user_id: N }`) shapes.
fn extract_peer(map: &serde_json::Map<String, Value>) -> Option<String> {
    for key in [
        "peerId",
        "peer_id",
        "peer",
        "dialog_peer_id",
        "dialogPeerId",
    ] {
        if let Some(v) = map.get(key) {
            if let Some(s) = num_to_str(v) {
                return Some(s);
            }
            if let Some(obj) = v.as_object() {
                for id_key in [
                    "user_id",
                    "userId",
                    "chat_id",
                    "chatId",
                    "channel_id",
                    "channelId",
                ] {
                    if let Some(id) = obj.get(id_key).and_then(num_to_str) {
                        return Some(id);
                    }
                }
            }
        }
    }
    None
}

/// Pull the sender identifier. Falls back to empty when not present (e.g.
/// service messages, channel posts without an explicit author).
fn extract_sender(map: &serde_json::Map<String, Value>) -> Option<String> {
    for key in ["fromId", "from_id", "fromID", "sender_id", "senderId"] {
        if let Some(v) = map.get(key) {
            if let Some(s) = num_to_str(v) {
                return Some(s);
            }
            if let Some(obj) = v.as_object() {
                for id_key in ["user_id", "userId", "channel_id", "channelId"] {
                    if let Some(id) = obj.get(id_key).and_then(num_to_str) {
                        return Some(id);
                    }
                }
            }
        }
    }
    None
}

/// A JSON `Value` viewed as an integer-ish id, serialised as a string so
/// it keys maps uniformly regardless of original encoding (int vs string).
fn num_to_str(v: &Value) -> Option<String> {
    match v {
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Some(i.to_string())
            } else {
                n.as_f64().map(|f| format!("{f}"))
            }
        }
        Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else if trimmed.chars().all(|c| c.is_ascii_digit() || c == '-') {
                Some(trimmed.to_string())
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Heuristic: a map key that's all digits (optionally negative) and 4+
/// chars long is plausibly a peer id (Telegram ids are large).
fn looks_like_peer_key(k: &str) -> bool {
    let bytes = k.as_bytes();
    if bytes.len() < 4 {
        return false;
    }
    let (first, rest) = bytes.split_first().unwrap();
    let starts_ok = first.is_ascii_digit() || *first == b'-';
    starts_ok && rest.iter().all(|b| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_dump() -> super::super::idb::IdbDump {
        super::super::idb::IdbDump::default()
    }

    #[test]
    fn extracts_message_shape() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "tweb".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "messages".into(),
                count: 1,
                records: vec![json!({
                    "id": 42,
                    "date": 1_712_345_678_i64,
                    "message": "hello world",
                    "peerId": 123456789_i64,
                    "fromId": 987654321_i64,
                })],
                error: None,
            }],
            error: None,
        });
        let h = harvest(&dump);
        assert_eq!(h.messages.len(), 1);
        assert_eq!(h.messages[0].peer, "123456789");
        assert_eq!(h.messages[0].sender, "987654321");
        assert_eq!(h.messages[0].text, "hello world");
        assert_eq!(h.messages[0].date, 1_712_345_678);
    }

    #[test]
    fn extracts_message_with_tl_peer_shape() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "tweb".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "messages".into(),
                count: 1,
                records: vec![json!({
                    "date": 1_712_345_678_i64,
                    "message": "channel post",
                    "peerId": { "_": "peerChannel", "channel_id": 555 },
                    "fromId": { "_": "peerUser", "user_id": 777 },
                })],
                error: None,
            }],
            error: None,
        });
        let h = harvest(&dump);
        assert_eq!(h.messages.len(), 1);
        assert_eq!(h.messages[0].peer, "555");
        assert_eq!(h.messages[0].sender, "777");
    }

    #[test]
    fn picks_up_user_and_chat_directories() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "tweb".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "state".into(),
                count: 1,
                records: vec![json!({
                    "users": [
                        { "id": 111, "first_name": "Ada", "last_name": "Lovelace" },
                        { "id": 222, "username": "babbage" },
                        { "id": 333, "first_name": "Me", "self": true }
                    ],
                    "chats": [
                        { "id": 444, "title": "Rust Lang" }
                    ]
                })],
                error: None,
            }],
            error: None,
        });
        let h = harvest(&dump);
        assert_eq!(h.users.get("111").map(String::as_str), Some("Ada Lovelace"));
        assert_eq!(h.users.get("222").map(String::as_str), Some("babbage"));
        assert_eq!(h.users.get("333").map(String::as_str), Some("Me"));
        assert_eq!(h.chats.get("444").map(String::as_str), Some("Rust Lang"));
        assert_eq!(h.self_id.as_deref(), Some("333"));
    }

    #[test]
    fn groups_messages_under_peer_key_hint() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "tweb".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "dialogs".into(),
                count: 1,
                records: vec![json!({
                    "999888777": [
                        { "date": 1_712_345_678_i64, "message": "hi", "fromId": 111 }
                    ]
                })],
                error: None,
            }],
            error: None,
        });
        let h = harvest(&dump);
        assert_eq!(h.messages.len(), 1);
        assert_eq!(h.messages[0].peer, "999888777");
    }

    #[test]
    fn rejects_implausible_dates() {
        let mut dump = empty_dump();
        dump.dbs.push(super::super::idb::IdbDb {
            name: "tweb".into(),
            stores: vec![super::super::idb::IdbStore {
                name: "weird".into(),
                count: 1,
                records: vec![json!({
                    "date": 42,
                    "message": "nope",
                    "peerId": 1,
                })],
                error: None,
            }],
            error: None,
        });
        let h = harvest(&dump);
        assert_eq!(h.messages.len(), 0);
    }
}
