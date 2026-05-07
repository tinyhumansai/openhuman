//! Request / response types for the `meet` domain.
//!
//! The `meet` domain captures the user's intent to have the agent join a
//! Google Meet call as an anonymous guest. The actual webview lifecycle is
//! handled by the Tauri shell — core's role is to validate the request,
//! mint a stable `request_id`, and emit a domain event so any interested
//! observer (frontend status pill, future audit log, the Tauri shell over
//! the socket bridge) can react to it.

use serde::{Deserialize, Serialize};

/// Inputs to `openhuman.meet_join_call`.
#[derive(Debug, Clone, Deserialize)]
pub struct MeetJoinCallRequest {
    /// Full Google Meet URL the agent should join, e.g.
    /// `https://meet.google.com/abc-defg-hij`.
    pub meet_url: String,
    /// Display name used by the agent when prompted by Meet's
    /// "Your name" field. Required because guest joins always need a name.
    pub display_name: String,
}

/// Outputs from `openhuman.meet_join_call`.
#[derive(Debug, Clone, Serialize)]
pub struct MeetJoinCallResponse {
    /// True when the request was accepted and a `request_id` was minted.
    pub ok: bool,
    /// Stable identifier for the join attempt. The Tauri shell uses this
    /// as the per-call data-directory and webview-window label so multiple
    /// concurrent calls don't collide.
    pub request_id: String,
    /// Echoed normalized URL, useful so the frontend can confirm what was
    /// accepted and surface it in the call list/UI.
    pub meet_url: String,
    /// Echoed display name for the same reason.
    pub display_name: String,
}
