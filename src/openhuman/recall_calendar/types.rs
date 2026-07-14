//! Serde types for the `recall_calendar` domain — the backend-proxied
//! Recall.ai Calendar V1 surface exposed at `openhuman.recall_calendar_*`.

use serde::{Deserialize, Serialize};

/// Connection status surfaced to the settings UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecallCalendarStatus {
    /// Whether the backend has the Recall calendar integration enabled
    /// (`RECALL_CALENDAR_ENABLED`). When `false` the UI hides the connect tile.
    pub enabled: bool,
    /// Whether this user has a connected Google Calendar.
    pub connected: bool,
    /// Connected calendar email, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Result of the connect op — the Google OAuth consent URL to open.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallCalendarConnect {
    /// Google OAuth consent URL the client opens in the browser.
    #[serde(rename = "connectUrl")]
    pub connect_url: String,
}

/// Result of the disconnect op.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallCalendarDisconnect {
    pub disconnected: bool,
}

/// A single upcoming meeting, already normalized by the backend's
/// `/agent-integrations/recall-calendar/meetings` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecallMeeting {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(rename = "meetingUrl", default)]
    pub meeting_url: Option<String>,
    /// RFC3339 start time.
    #[serde(rename = "startTime", default)]
    pub start_time: Option<String>,
    /// RFC3339 end time.
    #[serde(rename = "endTime", default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Populated only if Recall itself scheduled a bot (should stay empty in
    /// our detection-only model — we schedule via the existing bot path).
    #[serde(rename = "botId", default)]
    pub bot_id: Option<String>,
}

/// Envelope of the meetings endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallMeetingsResponse {
    #[serde(default)]
    pub meetings: Vec<RecallMeeting>,
}
