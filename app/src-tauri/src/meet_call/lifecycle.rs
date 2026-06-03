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

/// Map a `meet_scanner` error string + phase hint to a `ReasonCode`.
///
/// The substring matching is intentionally loose — the scanner builds
/// timeout messages via `format!` with the target text inlined, so we
/// look for the *target* (`"Your name"`, `"Ask to join"`, `"Leave-call"`)
/// rather than the framing words. On no match, fall back to the
/// phase-default so support always has *something* grep-able.
pub fn classify_scanner_err(err: &str, phase_hint: Phase) -> ReasonCode {
    if err.contains("Leave-call") || err.contains("admission") {
        return ReasonCode::AdmissionTimeout;
    }
    if err.contains("Your name") {
        return ReasonCode::NameInputTimeout;
    }
    if err.contains("Ask to join") {
        return ReasonCode::AskToJoinTimeout;
    }
    match phase_hint {
        Phase::Joining | Phase::AwaitingAdmission => ReasonCode::AskToJoinTimeout,
        Phase::Joined => ReasonCode::AdmissionTimeout,
    }
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

    #[test]
    fn classify_admission_timeout_from_substring() {
        let err = "timeout (120s) waiting for Leave-call affordance";
        assert_eq!(
            classify_scanner_err(err, Phase::Joined),
            ReasonCode::AdmissionTimeout
        );
    }

    #[test]
    fn classify_name_input_timeout_from_substring() {
        // wait_and_click variants embed the target text — defensive
        // matching against the literal `"Your name"` substring keeps
        // the helper robust to future format string tweaks.
        let err = "timeout typing into Your name input";
        assert_eq!(
            classify_scanner_err(err, Phase::AwaitingAdmission),
            ReasonCode::NameInputTimeout
        );
    }

    #[test]
    fn classify_ask_to_join_timeout_from_substring() {
        let err = "timeout finding text node 'Ask to join'";
        assert_eq!(
            classify_scanner_err(err, Phase::AwaitingAdmission),
            ReasonCode::AskToJoinTimeout
        );
    }

    #[test]
    fn classify_falls_back_to_phase_default_when_no_match() {
        // Unknown error text → fall back to the phase-default
        // ReasonCode so we never panic and always have something
        // grep-able for support.
        assert_eq!(
            classify_scanner_err("network unreachable", Phase::AwaitingAdmission),
            ReasonCode::AskToJoinTimeout
        );
        assert_eq!(
            classify_scanner_err("network unreachable", Phase::Joined),
            ReasonCode::AdmissionTimeout
        );
        assert_eq!(
            classify_scanner_err("network unreachable", Phase::Joining),
            ReasonCode::AskToJoinTimeout
        );
    }
}
