//! Data shapes returned by the Gmail API commands.
//!
//! Keep these stable across ops — the UI / agent consumers rely on the
//! shape, not the op that produced it. Owned `String` fields + `Vec<_>`
//! throughout so they serialize cleanly over the Tauri IPC bridge.

use serde::{Deserialize, Serialize};

/// Gmail label as scraped from the sidebar DOM.
///
/// Known limitations:
///
/// * `id` is currently set to the **display name**, not Gmail's internal
///   stable id (`INBOX`, `STARRED`, `Label_123`, …). DOM scraping can't
///   recover those without either Network MITM of the sync endpoints
///   or an authenticated API call. Treat `id` as an opaque display key
///   within the webview_apis surface; callers that need stable ids for
///   downstream Gmail API calls must wait for the Network-interception
///   follow-up tracked in the plan.
/// * `kind` is derived from an English name table in `reads.rs`. Users
///   on non-English Gmail locales will see every label classified as
///   `"user"` until localised detection lands.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GmailLabel {
    pub id: String,
    pub name: String,
    /// `"system"` (INBOX, SENT, TRASH, STARRED, …) or `"user"`.
    pub kind: String,
    /// Unread count if surfaced in the sidebar, else `None`.
    pub unread: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub from: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    /// Plain-text body if available. HTML bodies are not decoded here.
    pub body: Option<String>,
    /// Unix millis of the message's internal date, when surfaced.
    pub date_ms: Option<i64>,
    pub labels: Vec<String>,
    pub unread: bool,
}

#[allow(dead_code)] // returned by get_thread once wired; shape is stable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GmailThread {
    pub id: String,
    pub subject: Option<String>,
    pub messages: Vec<GmailMessage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GmailSendRequest {
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    #[serde(default)]
    pub bcc: Vec<String>,
    pub subject: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SendAck {
    pub message_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Ack {
    pub status: String,
}
