//! Request / response types for the `agent_meetings` domain.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Meeting session types (PR-0: #3506)
// ---------------------------------------------------------------------------

/// Opaque meeting identifier correlating calendar event → join → transcript → thread.
pub type MeetingId = String;

/// How the meeting join was triggered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoJoinSource {
    Calendar,
    Manual,
    Api,
}

/// Lifecycle state of a meeting session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeetingSessionStatus {
    /// Scheduled / awaiting join.
    Pending,
    /// Bot has joined the call.
    Joined,
    /// Call is in progress with active transcription.
    Active,
    /// Call has ended.
    Ended,
}

/// A single tracked meeting session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSession {
    pub id: MeetingId,
    pub meet_url: String,
    pub title: Option<String>,
    pub calendar_event_id: Option<String>,
    pub status: MeetingSessionStatus,
    pub source: AutoJoinSource,
    pub thread_id: Option<String>,
    pub transcript_received: bool,
    pub summary_generated: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// Kind of action item extracted from a meeting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionItemKind {
    /// Can be executed via a connected tool (requires approval).
    Executable,
    /// Informational only — no connected tool available.
    Advisory,
}

/// A single action item extracted from a meeting transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionItem {
    pub description: String,
    pub kind: ActionItemKind,
    pub tool_name: Option<String>,
    pub assignee: Option<String>,
}

/// Structured post-call summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeetingSummary {
    pub meeting_id: MeetingId,
    pub headline: String,
    pub key_points: Vec<String>,
    pub action_items: Vec<ActionItem>,
    pub generated_at_ms: u64,
}

// ---------------------------------------------------------------------------
// Existing backend RPC types
// ---------------------------------------------------------------------------

/// Optional Rive animation color overrides.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RiveColors {
    #[serde(default)]
    pub primary_color: Option<String>,
    #[serde(default)]
    pub secondary_color: Option<String>,
}

/// One mascot slot for a multi-mascot backend meeting (issue #4277).
/// Slot 0 = primary speaker, slot 1 = secondary. The backend bot renders
/// both mascots side-by-side, alternates the speaking slot per reply, and
/// synthesizes each slot's replies with `voice_id` (falling back to its
/// configured default when absent).
///
/// Deserialize-only: the RPC input arrives snake_case (`mascot_id`,
/// `rive_colors`, `voice_id`); the outbound `bot:join` payload is built by
/// hand in `ops.rs` (camelCase), so this struct is never serialized.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMascotSlot {
    /// Rive mascot id (e.g. "yellow", "toshi").
    pub mascot_id: String,
    /// Optional human-facing mascot name (e.g. "Toshi", "Tiny"), taken from the
    /// manifest. Drives name-addressed routing (#4277 follow-up): a participant
    /// who says "Hey Toshi …" is routed to this slot instead of the mechanical
    /// alternation. Absent → that slot is not name-addressable.
    #[serde(default)]
    pub name: Option<String>,
    /// Optional per-mascot color palette overrides.
    #[serde(default)]
    pub rive_colors: Option<RiveColors>,
    /// Optional per-mascot ElevenLabs voice id.
    #[serde(default)]
    pub voice_id: Option<String>,
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
    /// Legacy single-mascot field; still honored when `mascots` is absent.
    #[serde(default)]
    pub mascot_id: Option<String>,
    /// Optional Rive mascot color palette overrides (single-mascot path).
    #[serde(default)]
    pub rive_colors: Option<RiveColors>,
    /// Dual-mascot config (issue #4277). When present (2 slots) the backend
    /// renders both mascots and alternates the speaker per reply, using each
    /// slot's `voice_id`. Absent / single-element falls back to the legacy
    /// single-mascot `mascot_id` + `rive_colors` behavior.
    #[serde(default)]
    pub mascots: Option<Vec<BackendMascotSlot>>,
    /// Only respond to this participant's messages (empty/absent = respond to everyone).
    #[serde(default)]
    pub respond_to_participant: Option<String>,
    /// Wake phrase the participant must say before the bot responds.
    #[serde(default)]
    pub wake_phrase: Option<String>,
    /// Opaque correlation id echoed on all `bot:*` events for this session.
    #[serde(default)]
    pub correlation_id: Option<String>,
    /// When `true`, the bot joins in listen-only mode (no microphone, no replies).
    #[serde(default)]
    pub listen_only: Option<bool>,
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

/// Inputs to `openhuman.agent_meetings_speak`.
#[derive(Debug, Clone, Deserialize)]
pub struct BackendMeetSpeakRequest {
    pub text: String,
    #[serde(default)]
    pub correlation_id: Option<String>,
}

/// Inputs to `openhuman.agent_meetings_generate_summary`.
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateSummaryRequest {
    /// Meeting/call id. For backend Meet calls this is the request_id used by
    /// the recent-calls detail store.
    pub meeting_id: String,
}

/// Outputs from `openhuman.agent_meetings_generate_summary`.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateSummaryResponse {
    pub ok: bool,
    pub thread_id: String,
}

// ---------------------------------------------------------------------------
// meet_list_upcoming RPC types
// ---------------------------------------------------------------------------

/// Inputs to `openhuman.meet_list_upcoming`.
#[derive(Debug, Clone, Deserialize)]
pub struct ListUpcomingRequest {
    /// How many minutes ahead to look for meetings. Defaults to 480 (8 hours).
    #[serde(default)]
    pub lookahead_minutes: Option<u32>,
    /// Maximum number of meetings to return. Defaults to 20.
    #[serde(default)]
    pub limit: Option<u32>,
}

/// One upcoming calendar meeting that has a conferencing link.
#[derive(Debug, Clone, Serialize)]
pub struct UpcomingMeeting {
    /// Calendar provider event id (stable dedupe key).
    pub calendar_event_id: String,
    /// Human-readable meeting title (from calendar event summary).
    pub title: String,
    /// Start time as Unix milliseconds.
    pub start_time_ms: u64,
    /// End time as Unix milliseconds.
    pub end_time_ms: u64,
    /// Conferencing URL (Google Meet, Zoom, Teams, Webex).
    pub meet_url: Option<String>,
    /// Platform slug inferred from the URL host: gmeet, zoom, teams, webex.
    pub platform: Option<String>,
    /// Number of attendees listed on the calendar event.
    pub participant_count: Option<u32>,
    /// Organizer display name or email, if present.
    pub organizer: Option<String>,
    /// Join policy string: "auto" | "ask" | "skip" (mapped from MeetConfig.auto_join_policy).
    pub join_policy: String,
    /// Source integration slug, e.g. "googlecalendar".
    pub calendar_source: String,
}

/// Response from `openhuman.meet_list_upcoming`.
#[derive(Debug, Clone, Serialize)]
pub struct ListUpcomingResponse {
    pub ok: bool,
    pub meetings: Vec<UpcomingMeeting>,
}

// ---------------------------------------------------------------------------
// Phase 3 per-event policy RPC types
// ---------------------------------------------------------------------------

/// Request for `openhuman.meet_set_event_policy`.
#[derive(Debug, Deserialize)]
pub struct SetEventPolicyRequest {
    pub calendar_event_id: String,
    /// "auto" | "ask" | "skip"
    pub policy: String,
}

/// Response for `openhuman.meet_set_event_policy`.
#[derive(Debug, Serialize)]
pub struct SetEventPolicyResponse {
    pub ok: bool,
}

/// Request for `openhuman.meet_get_event_policies`.
#[derive(Debug, Deserialize)]
pub struct GetEventPoliciesRequest {
    pub calendar_event_ids: Vec<String>,
}

/// Response for `openhuman.meet_get_event_policies`.
#[derive(Debug, Serialize)]
pub struct GetEventPoliciesResponse {
    pub ok: bool,
    pub policies: std::collections::HashMap<String, String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeting_session_serde_round_trip() {
        let session = MeetingSession {
            id: "meet-123".into(),
            meet_url: "https://meet.google.com/abc-defg-hij".into(),
            title: Some("Standup".into()),
            calendar_event_id: Some("cal-456".into()),
            status: MeetingSessionStatus::Active,
            source: AutoJoinSource::Calendar,
            thread_id: Some("thread-789".into()),
            transcript_received: true,
            summary_generated: false,
            created_at_ms: 1000,
            updated_at_ms: 2000,
        };
        let json = serde_json::to_string(&session).unwrap();
        let back: MeetingSession = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "meet-123");
        assert_eq!(back.status, MeetingSessionStatus::Active);
        assert_eq!(back.source, AutoJoinSource::Calendar);
        assert!(back.transcript_received);
    }

    #[test]
    fn action_item_kinds_serialize() {
        let exec = ActionItemKind::Executable;
        let adv = ActionItemKind::Advisory;
        assert_eq!(serde_json::to_string(&exec).unwrap(), "\"executable\"");
        assert_eq!(serde_json::to_string(&adv).unwrap(), "\"advisory\"");
    }

    #[test]
    fn join_request_with_correlation_fields() {
        let json = serde_json::json!({
            "meet_url": "https://meet.google.com/x",
            "correlation_id": "meet-123",
            "listen_only": true
        });
        let req: BackendMeetJoinRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.correlation_id.as_deref(), Some("meet-123"));
        assert_eq!(req.listen_only, Some(true));
    }

    #[test]
    fn join_request_backward_compat_no_new_fields() {
        let json = serde_json::json!({
            "meet_url": "https://meet.google.com/x"
        });
        let req: BackendMeetJoinRequest = serde_json::from_value(json).unwrap();
        assert!(req.correlation_id.is_none());
        assert!(req.listen_only.is_none());
    }
}
