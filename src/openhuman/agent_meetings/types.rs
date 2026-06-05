//! Request / response types for the `agent_meetings` domain.

use serde::{Deserialize, Serialize};

/// Optional Rive animation color overrides.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RiveColors {
    #[serde(default)]
    pub primary_color: Option<String>,
    #[serde(default)]
    pub secondary_color: Option<String>,
}

/// Inputs to `openhuman.agent_meetings_join`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeetJoinRequest {
    pub meet_url: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    /// Display name for the AI agent (shown in bot replies and LLM system prompt).
    #[serde(default)]
    pub agent_name: Option<String>,
    /// Custom system prompt for the meeting LLM. `{{AGENT_NAME}}` is replaced server-side.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Selects which Rive mascot appears in the meeting (e.g. "yellow", "blue").
    /// Defaults to the backend's configured default mascot when omitted.
    #[serde(default)]
    pub mascot_id: Option<String>,
    /// Optional Rive mascot color palette overrides.
    #[serde(default)]
    pub rive_colors: Option<RiveColors>,
    /// Only respond to this participant's messages (empty/absent = respond to everyone).
    /// Case-insensitive substring match against the speaker name in the transcript.
    #[serde(default)]
    pub respond_to_participant: Option<String>,
    /// Wake phrase the participant must say before the bot responds.
    /// When set, captions without this phrase are silently dropped.
    /// The phrase is stripped from the text before it reaches the LLM.
    #[serde(default)]
    pub wake_phrase: Option<String>,
}

/// Outputs from `openhuman.agent_meetings_join`.
#[derive(Debug, Clone, Serialize)]
pub struct BackendMeetJoinResponse {
    pub ok: bool,
    pub meet_url: String,
    pub platform: String,
}

/// Inputs to `openhuman.agent_meetings_leave`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeetLeaveRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

/// Inputs to `openhuman.agent_meetings_harness_response`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeetHarnessResponseRequest {
    pub result: String,
}
