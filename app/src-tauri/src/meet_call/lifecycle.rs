//! Per-call lifecycle beacons for the Meet join flow.
//!
//! Emitted from the Tauri shell (`meet_call`, `meet_scanner`, `meet_audio`)
//! so the frontend can render an actionable terminal-failure toast and
//! `grep "[meet-lifecycle]"` reconstructs one call's story from the log.
//! See [`docs/superpowers/specs/2026-06-03-meet-call-lifecycle-diagnostics-design.md`].

use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::meet_call::MeetCallState;

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

/// Emit a `meet-call:phase` event for a non-terminal lifecycle transition.
///
/// Non-idempotent on purpose — phase transitions can legitimately fire
/// twice if the scanner retries internally. The frontend's listener
/// only cares about the *latest* phase before terminal, so duplicates
/// are harmless.
pub fn emit_phase<R: Runtime>(
    app: &AppHandle<R>,
    request_id: &str,
    phase: Phase,
    detail: Option<&str>,
) {
    log::info!(
        "[meet-lifecycle] phase={} request_id={request_id} detail={}",
        serde_json::to_value(phase)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "?".into()),
        detail.unwrap_or("")
    );
    if let Err(err) = app.emit(
        "meet-call:phase",
        json!({
            "request_id": request_id,
            "phase": phase,
            "detail": detail,
        }),
    ) {
        log::debug!("[meet-lifecycle] emit phase failed: {err}");
    }
}

/// Emit a `meet-call:failed` event with one-shot per-`request_id` dedup.
///
/// Consults [`MeetCallState::mark_terminated`]; a second call for the
/// same `request_id` is a no-op + debug log. `message` is the
/// localized human string the frontend can hand straight to the toast.
pub fn emit_failed<R: Runtime>(
    app: &AppHandle<R>,
    request_id: &str,
    phase: Phase,
    reason: ReasonCode,
    message: &str,
) {
    let state = match app.try_state::<MeetCallState>() {
        Some(s) => s,
        None => {
            log::warn!(
                "[meet-lifecycle] emit_failed skipped (state missing) request_id={request_id}"
            );
            return;
        }
    };
    if !state.mark_terminated(request_id) {
        log::debug!(
            "[meet-lifecycle] emit_failed deduped request_id={request_id} reason={:?}",
            reason
        );
        return;
    }
    log::warn!(
        "[meet-lifecycle] failed phase={} reason={} request_id={request_id} message={message}",
        serde_json::to_value(phase)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "?".into()),
        serde_json::to_value(reason)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_else(|| "?".into()),
    );
    if let Err(err) = app.emit(
        "meet-call:failed",
        json!({
            "request_id": request_id,
            "phase": phase,
            "reason_code": reason,
            "message": message,
        }),
    ) {
        log::debug!("[meet-lifecycle] emit failed failed: {err}");
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
