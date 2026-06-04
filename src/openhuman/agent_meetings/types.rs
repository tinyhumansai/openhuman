//! Request / response types for the `agent_meetings` domain.

use serde::{Deserialize, Serialize};

/// Inputs to `openhuman.agent_meetings_join`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeetJoinRequest {
    pub meet_url: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
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
