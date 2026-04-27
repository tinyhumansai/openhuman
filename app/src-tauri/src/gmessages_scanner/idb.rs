//! Google Messages Web `bugle_db` IndexedDB schema + normalization.
//!
//! Schema knowledge is taken from publicly documented reverse-engineering
//! of the Google Messages Web client (the `mautrix-gmessages` project and
//! the Google Messages Web source itself). No code is copied —
//! only the factual store / key shape, which is not copyrightable.
//!
//! Stores we care about:
//!   * `conversations` — thread metadata (id, participant ids, name)
//!   * `messages`      — individual SMS/RCS rows
//!   * `participants`  — participant id → contact name resolution
//!
//! Stores we deliberately skip:
//!   * `settings`, `drafts`, `attachments-cache` — not needed for recall.
//!
//! This module only holds schema + normalization. The CDP walk that
//! actually calls `IndexedDB.requestData` will live alongside the WhatsApp
//! scanner's CDP plumbing once we lift a shared `cdp` module — see the
//! TODO in `mod.rs`.

use std::collections::HashMap;

use serde_json::Value;

/// `bugle_db` database name. Stable since ~2022 per mautrix-gmessages
/// history; Google has not shipped a schema rename in the tracked window.
pub const DATABASE_NAME: &str = "bugle_db";
pub const STORE_CONVERSATIONS: &str = "conversations";
pub const STORE_MESSAGES: &str = "messages";
pub const STORE_PARTICIPANTS: &str = "participants";

/// Normalized message row emitted to the memory-doc pipeline.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub id: String,
    pub thread_id: Option<String>,
    /// `None` when the message is outbound (sent by the user).
    pub sender_id: Option<String>,
    pub from_me: bool,
    /// Plain UTF-8 body. Attachments / reactions collapse to empty string
    /// at normalization — callers render them as `[non-text]`.
    pub text: String,
    pub timestamp_unix: i64,
    /// "sms", "rcs", "mms", etc. Preserved for downstream filters.
    pub message_type: Option<String>,
}

/// Normalized conversation (thread) metadata.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Conversation {
    pub thread_id: String,
    pub display_name: Option<String>,
    pub participant_ids: Vec<String>,
}

/// Participant-id → display-name map. Populated from the `participants`
/// store; used by `format_transcript` to render human-readable senders.
#[derive(Debug, Default, Clone)]
pub struct ParticipantMap {
    inner: HashMap<String, String>,
}

impl ParticipantMap {
    pub fn insert(&mut self, id: String, name: String) {
        self.inner.insert(id, name);
    }

    pub fn display_name(&self, id: &str) -> Option<String> {
        self.inner.get(id).cloned()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

/// Convert a raw JSON row from the `messages` object store into our
/// normalized shape. Returns `None` if required fields are missing or
/// malformed — we log + skip rather than failing the entire walk.
///
/// Expected bugle_db fields (observed, documented in mautrix-gmessages):
///   * `messageId` (string) — primary key
///   * `conversationId` (string)
///   * `senderId` (string, absent for outgoing)
///   * `messageStatus` (object with `status` int; outgoing statuses 2/4/6)
///   * `text` (string; may be absent for attachment-only)
///   * `timestamp` (int; microseconds since unix epoch)
///   * `messageType` (string: "SMS", "RCS", etc.)
pub fn normalize_message(raw: &Value) -> Option<Message> {
    let id = raw.get("messageId")?.as_str()?.to_string();
    let thread_id = raw
        .get("conversationId")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let sender_id = raw
        .get("senderId")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let from_me = is_outgoing(raw);
    let text = raw
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // bugle_db timestamps are microseconds since unix epoch. Guard against
    // the legacy-seconds form (< 10^12 = before year 33700 in micros,
    // practically any real timestamp is in the 10^15 range).
    let timestamp_unix = raw.get("timestamp").and_then(|v| v.as_i64()).map(|t| {
        if t > 1_000_000_000_000 {
            t / 1_000_000
        } else {
            t
        }
    })?;
    let message_type = raw
        .get("messageType")
        .and_then(|v| v.as_str())
        .map(|s| s.to_ascii_lowercase());

    Some(Message {
        id,
        thread_id,
        sender_id: if from_me { None } else { sender_id },
        from_me,
        text,
        timestamp_unix,
        message_type,
    })
}

/// Heuristic: bugle_db marks outgoing messages with a `messageStatus`
/// object whose `status` is in {2 (OUTGOING_DELIVERED), 4 (OUTGOING_READ),
/// 6 (OUTGOING_FAILED)} or an explicit boolean `isOutgoing` on newer
/// schemas. Fall back to `senderId == null` which is also a reliable
/// signal on older writes.
fn is_outgoing(raw: &Value) -> bool {
    if let Some(b) = raw.get("isOutgoing").and_then(|v| v.as_bool()) {
        return b;
    }
    if let Some(status) = raw
        .get("messageStatus")
        .and_then(|s| s.get("status"))
        .and_then(|v| v.as_i64())
    {
        // Status codes 1-9 are outgoing; 10+ are incoming (OUTGOING_* vs
        // INCOMING_* in the bugle_db protobuf enum). Exact values per
        // mautrix-gmessages' `libgm/events/types.go`.
        return (1..=9).contains(&status);
    }
    raw.get("senderId")
        .map(|v| v.is_null() || v.as_str().is_some_and(str::is_empty))
        .unwrap_or(false)
}

/// Normalize a `conversations` store row.
pub fn normalize_conversation(raw: &Value) -> Option<Conversation> {
    let thread_id = raw.get("conversationId")?.as_str()?.to_string();
    let display_name = raw.get("name").and_then(|v| v.as_str()).map(str::to_string);
    let participant_ids = raw
        .get("participantIds")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(Conversation {
        thread_id,
        display_name,
        participant_ids,
    })
}

/// Normalize a `participants` store row into `(id, name)`.
pub fn normalize_participant(raw: &Value) -> Option<(String, String)> {
    let id = raw.get("participantId")?.as_str()?.to_string();
    let name = raw
        .get("fullName")
        .or_else(|| raw.get("firstName"))
        .or_else(|| raw.get("displayName"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if name.is_empty() {
        None
    } else {
        Some((id, name))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn normalize_incoming_sms_row() {
        let raw = json!({
            "messageId": "msg-1",
            "conversationId": "thread-1",
            "senderId": "+15551234567",
            "text": "hello",
            "timestamp": 1_700_000_000_000_000i64,
            "messageType": "SMS",
            "messageStatus": { "status": 100 },
        });
        let m = normalize_message(&raw).expect("normalize ok");
        assert_eq!(m.id, "msg-1");
        assert_eq!(m.thread_id.as_deref(), Some("thread-1"));
        assert_eq!(m.sender_id.as_deref(), Some("+15551234567"));
        assert!(!m.from_me);
        assert_eq!(m.text, "hello");
        assert_eq!(m.timestamp_unix, 1_700_000_000);
        assert_eq!(m.message_type.as_deref(), Some("sms"));
    }

    #[test]
    fn normalize_outgoing_row_sets_from_me_and_blanks_sender() {
        let raw = json!({
            "messageId": "msg-2",
            "conversationId": "thread-1",
            "senderId": "+15559998888",
            "text": "yo",
            "timestamp": 1_700_000_005_000_000i64,
            "messageStatus": { "status": 4 },
        });
        let m = normalize_message(&raw).expect("normalize ok");
        assert!(m.from_me, "status=4 is OUTGOING_READ");
        assert!(m.sender_id.is_none(), "outgoing rows blank the sender");
    }

    #[test]
    fn normalize_accepts_legacy_second_precision_timestamp() {
        let raw = json!({
            "messageId": "msg-3",
            "conversationId": "thread-1",
            "senderId": "+15551234567",
            "text": "hi",
            "timestamp": 1_700_000_000i64,
            "messageStatus": { "status": 100 },
        });
        let m = normalize_message(&raw).expect("normalize ok");
        assert_eq!(m.timestamp_unix, 1_700_000_000);
    }

    #[test]
    fn normalize_skips_row_missing_required_fields() {
        let raw = json!({
            "conversationId": "thread-1",
            "text": "no id",
            "timestamp": 1_700_000_000_000_000i64,
        });
        assert!(normalize_message(&raw).is_none());
    }

    #[test]
    fn normalize_conversation_row_with_participants() {
        let raw = json!({
            "conversationId": "thread-1",
            "name": "Family Group",
            "participantIds": ["+15551234567", "+15559998888"],
        });
        let c = normalize_conversation(&raw).expect("normalize ok");
        assert_eq!(c.thread_id, "thread-1");
        assert_eq!(c.display_name.as_deref(), Some("Family Group"));
        assert_eq!(c.participant_ids.len(), 2);
    }

    #[test]
    fn normalize_participant_row_prefers_full_name() {
        let raw = json!({
            "participantId": "+15551234567",
            "fullName": "Alice Example",
            "firstName": "Alice",
        });
        let (id, name) = normalize_participant(&raw).expect("normalize ok");
        assert_eq!(id, "+15551234567");
        assert_eq!(name, "Alice Example");
    }

    #[test]
    fn normalize_participant_falls_back_to_first_name() {
        let raw = json!({
            "participantId": "+15551234567",
            "firstName": "Alice",
        });
        let (_, name) = normalize_participant(&raw).expect("normalize ok");
        assert_eq!(name, "Alice");
    }

    #[test]
    fn normalize_participant_returns_none_for_empty_name() {
        let raw = json!({
            "participantId": "+15551234567",
        });
        assert!(normalize_participant(&raw).is_none());
    }

    #[test]
    fn participant_map_roundtrip() {
        let mut pm = ParticipantMap::default();
        assert!(pm.is_empty());
        pm.insert("+15551234567".into(), "Alice".into());
        assert_eq!(pm.len(), 1);
        assert_eq!(pm.display_name("+15551234567").as_deref(), Some("Alice"));
        assert!(pm.display_name("unknown").is_none());
    }
}
