//! Data shapes returned by the Gmail API commands.
//!
//! Keep these stable across ops — the UI / agent consumers rely on the
//! shape, not the op that produced it. Owned `String` fields + `Vec<_>`
//! throughout so they serialize cleanly over the Tauri IPC bridge.

use serde::{Deserialize, Serialize};

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
