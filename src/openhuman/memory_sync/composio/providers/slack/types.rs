//! Canonical types for the Composio-backed Slack provider.
//!
//! These types are independent of the Composio/Slack API payload shape.
//! Parsing of raw JSON into these structs happens in
//! [`super::sync`]; everything downstream deals only with the
//! canonical types below.
//!
//! The old `Bucket` struct (6-hour UTC window) has been removed — the
//! memory tree's L0 seal cascade handles batching after PR #1348, so
//! the provider just collects all fetched messages and calls
//! `ingest_page_into_memory_tree` per channel.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A single message fetched from Slack's `conversations.history` or
/// `search.messages`.
///
/// The Slack API represents `ts` as a decimal string like
/// `"1714003200.123456"` where the integer part is Unix seconds and the
/// fractional part is a per-workspace message sequence. We retain the
/// original string in `ts_raw` so it can round-trip back to the API
/// (e.g. as the `oldest` cursor on the next poll, and as the permalink
/// suffix for provenance).
///
/// `channel_name`, `is_private`, `author_id`, and `permalink` are added
/// vs the old `memory::slack_ingestion::types::SlackMessage` because we no
/// longer carry a separate `SlackChannel` through the ingest path —
/// per-message context is self-contained.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlackMessage {
    /// Channel ID this message belongs to (e.g. `"C0123456"`).
    pub channel_id: String,
    /// Human-readable channel name (e.g. `"eng"`). Injected by the enricher
    /// from the channel directory; may be empty for search results whose
    /// channel was not listed.
    pub channel_name: String,
    /// `true` if this is a private channel the bot has been invited to.
    pub is_private: bool,
    /// Resolved display name of the author. Falls back to the raw user id
    /// when the user directory doesn't have an entry for this id.
    pub author: String,
    /// Raw Slack user id (e.g. `"U01234"`). Retained alongside the resolved
    /// `author` so downstream code can still look up or log the stable id.
    pub author_id: String,
    /// Message body (plain text; may contain Slack-flavoured markdown).
    pub text: String,
    /// Canonical timestamp derived from `ts_raw`.
    pub timestamp: DateTime<Utc>,
    /// Raw Slack `ts` string (used for API cursors + archive URLs).
    pub ts_raw: String,
    /// Root thread `ts` if this message is a reply; `None` for top-level
    /// messages. Retained for future thread-aware ingestion.
    pub thread_ts: Option<String>,
    /// Resolved HTTPS permalink, if Composio includes it in the response.
    /// Falls back to the `slack://archives/…` scheme in ingest.
    pub permalink: Option<String>,
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
