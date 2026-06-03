//! Per-call lifecycle beacons for the Meet join flow.
//!
//! Emitted from the Tauri shell (`meet_call`, `meet_scanner`, `meet_audio`)
//! so the frontend can render an actionable terminal-failure toast and
//! `grep "[meet-lifecycle]"` reconstructs one call's story from the log.
//! See [`docs/superpowers/specs/2026-06-03-meet-call-lifecycle-diagnostics-design.md`].

use serde::Serialize;

/// Coarse-grained per-call phase. Sub-phases stay in logs only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Joining,
    AwaitingAdmission,
    Joined,
}

/// Why a call entered a terminal failure state.
///
/// `InvalidUrl` / `WindowBuildFailed` / `Cancelled` are reserved for
/// log-symmetry — they surface via the rejected `meet_call_open_window`
/// RPC promise or via `meet-call:closed`, **not** as `meet-call:failed`
/// events. The other four are the event-emitted set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasonCode {
    InvalidUrl,
    WindowBuildFailed,
    NameInputTimeout,
    AskToJoinTimeout,
    AdmissionTimeout,
    AudioBindFailed,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_serializes_to_snake_case() {
        assert_eq!(serde_json::to_value(Phase::Joining).unwrap(), "joining");
        assert_eq!(
            serde_json::to_value(Phase::AwaitingAdmission).unwrap(),
            "awaiting_admission"
        );
        assert_eq!(serde_json::to_value(Phase::Joined).unwrap(), "joined");
    }

    #[test]
    fn reason_code_serializes_to_snake_case() {
        assert_eq!(
            serde_json::to_value(ReasonCode::InvalidUrl).unwrap(),
            "invalid_url"
        );
        assert_eq!(
            serde_json::to_value(ReasonCode::WindowBuildFailed).unwrap(),
            "window_build_failed"
        );
        assert_eq!(
            serde_json::to_value(ReasonCode::NameInputTimeout).unwrap(),
            "name_input_timeout"
        );
        assert_eq!(
            serde_json::to_value(ReasonCode::AskToJoinTimeout).unwrap(),
            "ask_to_join_timeout"
        );
        assert_eq!(
            serde_json::to_value(ReasonCode::AdmissionTimeout).unwrap(),
            "admission_timeout"
        );
        assert_eq!(
            serde_json::to_value(ReasonCode::AudioBindFailed).unwrap(),
            "audio_bind_failed"
        );
        assert_eq!(
            serde_json::to_value(ReasonCode::Cancelled).unwrap(),
            "cancelled"
        );
    }
}
