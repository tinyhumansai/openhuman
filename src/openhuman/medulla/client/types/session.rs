//! Durable session lifecycle types (/medulla/v1/sessions).
//!
//! Split from the parent types module. Field names use serde renames to match
//! the backend camelCase wire format exactly, and unknown fields are tolerated
//! so the client keeps working against newer server versions.

use serde::{Deserialize, Serialize};

/// Session lifecycle status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Idle,
    Archived,
    /// Any status not yet modelled by this client.
    ///
    /// Also the `Default`: a value constructed rather than decoded has not
    /// declared a status, and defaulting to `Active` would assert one.
    #[default]
    #[serde(other)]
    Other,
}

/// Message author role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
    #[serde(other)]
    Other,
}

/// Result of creating a session (`POST /medulla/v1/sessions`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionCreated {
    pub session_id: String,
}

/// Item in the session list (`GET /medulla/v1/sessions`).
#[derive(Debug, Clone, Deserialize, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub last_active_at: Option<i64>,
    pub status: SessionStatus,
    #[serde(default)]
    pub last_seq: Option<i64>,
}

/// Detailed session state (`GET /medulla/v1/sessions/:id`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionDetail {
    pub session_id: String,
    pub status: SessionStatus,
    #[serde(default)]
    pub last_cycle_id: Option<String>,
    #[serde(default)]
    pub last_seq: Option<i64>,
    #[serde(default)]
    pub event_seq: Option<i64>,
}

/// Result of archiving a session (`DELETE /medulla/v1/sessions/:id`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionArchived {
    pub session_id: String,
    pub status: SessionStatus,
}

/// Result of `POST /medulla/v1/sessions/:id/messages`.
///
/// The async (202) response carries `cycle_id`/`seq`; the sync (`?sync=1`)
/// response additionally carries `reply`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SendResult {
    pub cycle_id: String,
    pub seq: i64,
    #[serde(default)]
    pub reply: Option<String>,
}

/// A replayed message (`GET /medulla/v1/sessions/:id/messages`).
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub seq: i64,
    pub role: Role,
    pub body: String,
    #[serde(default)]
    pub ts: Option<i64>,
    #[serde(default)]
    pub cycle_id: Option<String>,
}

/// Result of `POST /medulla/v1/sessions/:id/abort`.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AbortResult {
    pub session_id: String,
    pub aborted: bool,
}
