//! Shared types for the Slack ingestion path.
//!
//! These are intentionally independent of the Slack Web API payload
//! shape. Parsing of Composio-wrapped JSON into these structs happens in
//! [`crate::openhuman::composio::providers::slack::sync`]; everything
//! downstream only deals with the canonical types below.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single message fetched from Slack's `conversations.history`.
///
/// The Slack API represents `ts` as a decimal string like `"1714003200.123456"`
/// where the integer part is Unix seconds and the fractional part is a
/// per-workspace message sequence. We retain the original string in `ts_raw`
/// so it can round-trip back to the API (e.g. as the `oldest` cursor on the
/// next poll, and as the permalink suffix for provenance).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackMessage {
    /// Channel ID this message belongs to (e.g. `"C0123456"`).
    pub channel_id: String,
    /// Slack user ID of the author (e.g. `"U01234"`). Empty if the message
    /// was a bot/system event we still want to retain (rare).
    pub author: String,
    /// Message body (plain text; may contain Slack-flavoured markdown).
    pub text: String,
    /// Canonical bucketing timestamp derived from `ts_raw` — Unix millis.
    pub timestamp: DateTime<Utc>,
    /// Raw Slack `ts` string (used for API cursors + permalinks).
    pub ts_raw: String,
    /// Root thread `ts` if this message is a reply; `None` for channel-level
    /// messages. Retained for future thread-aware ingestion (v2).
    pub thread_ts: Option<String>,
}

/// A Slack channel visible to the bot, as returned by `conversations.list`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackChannel {
    /// Channel ID (stable across renames).
    pub id: String,
    /// Human-readable name (e.g. `"eng"` → rendered as `"#eng"` in headers).
    /// May change if admins rename the channel.
    pub name: String,
    /// `true` if this is a private channel the bot has been invited to.
    pub is_private: bool,
}

/// A closed time window of messages ready for ingest.
///
/// Created by [`super::bucketer::split_closed`] when every 6-hour UTC
/// window older than `now - GRACE_PERIOD` is extracted from the buffer.
/// `messages` is non-empty by construction — empty buckets are dropped
/// before they reach this type.
#[derive(Clone, Debug)]
pub struct Bucket {
    /// Inclusive bucket start (wall-clock UTC aligned to 00/06/12/18).
    pub start: DateTime<Utc>,
    /// Exclusive bucket end (start + 6 hours).
    pub end: DateTime<Utc>,
    /// Messages that fall in `[start, end)`, in arrival order.
    pub messages: Vec<SlackMessage>,
}

// Per-channel cursors are stored by the Composio-backed SlackProvider
// via `composio::providers::sync_state::SyncState` (a JSON-encoded
// `BTreeMap<channel_id, epoch_secs>` in the `cursor` field). The
// standalone `SyncCursor` struct that previously lived here has been
// removed along with the SQLite cursor table — retention happens in
// the memory KV store now, keyed by (toolkit="slack", connection_id).
