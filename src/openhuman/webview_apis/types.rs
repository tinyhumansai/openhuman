//! Core-side mirror of the Gmail shapes returned by the bridge.
//!
//! These must stay wire-compatible with
//! `app/src-tauri/src/gmail/types.rs`. Kept as plain types here —
//! there's no domain logic attached yet, and the controller schemas
//! describe them via `TypeSchema::Object { … }` / `TypeSchema::Ref(…)`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GmailLabel {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub unread: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub from: Option<String>,
    #[serde(default)]
    pub to: Vec<String>,
    #[serde(default)]
    pub cc: Vec<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub body: Option<String>,
    pub date_ms: Option<i64>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub unread: bool,
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
