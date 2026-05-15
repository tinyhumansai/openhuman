//! Unit tests for `accessibility::permissions`.
//!
//! macOS-only FFI functions (`detect_accessibility_permission`,
//! `detect_screen_recording_permission`, `detect_input_monitoring_permission`,
//! `request_*`) call into Apple frameworks and cannot be exercised in a
//! cross-platform test binary without hardware. Tests here cover:
//!
//!  - `permission_to_str` — pure logic, always available.
//!  - `microphone_denied_message` — pure logic, always available.
//!  - `detect_permissions` — non-macOS fallback (unsupported states).
//!  - `detect_microphone_permission` — cross-platform probe guard.

use super::*;

// ── permission_to_str ─────────────────────────────────────────────────────

#[test]
fn permission_to_str_screen_recording() {
    assert_eq!(
        permission_to_str(PermissionKind::ScreenRecording),
        "screen_recording"
    );
}

#[test]
fn permission_to_str_accessibility() {
    assert_eq!(
        permission_to_str(PermissionKind::Accessibility),
        "accessibility"
    );
}

#[test]
fn permission_to_str_input_monitoring() {
    assert_eq!(
        permission_to_str(PermissionKind::InputMonitoring),
        "input_monitoring"
    );
}

#[test]
fn permission_to_str_microphone() {
    assert_eq!(permission_to_str(PermissionKind::Microphone), "microphone");
}

#[test]
fn permission_to_str_is_snake_case_and_nonempty() {
    for kind in [
        PermissionKind::ScreenRecording,
        PermissionKind::Accessibility,
        PermissionKind::InputMonitoring,
        PermissionKind::Microphone,
    ] {
        let s = permission_to_str(kind);
        assert!(
            !s.is_empty(),
            "permission_to_str should never return empty string"
        );
        // Convention: snake_case, no spaces
        assert!(
            !s.contains(' '),
            "permission string should not contain spaces: {s}"
        );
        assert_eq!(
            s,
            s.to_ascii_lowercase(),
            "permission string should be lowercase: {s}"
        );
    }
}

// ── microphone_denied_message ─────────────────────────────────────────────

#[test]
fn microphone_denied_message_is_nonempty() {
    let msg = microphone_denied_message();
    assert!(!msg.is_empty(), "denied message should not be empty");
}

#[test]
fn microphone_denied_message_is_human_readable() {
    let msg = microphone_denied_message().to_ascii_lowercase();
    // All platform messages mention "microphone" as a cue to the user.
    assert!(
        msg.contains("microphone"),
        "denied message should mention 'microphone': {msg}"
    );
}

// ── detect_permissions (non-macOS path) ──────────────────────────────────

/// On non-macOS platforms `detect_permissions` reports screen_recording,
/// accessibility, and input_monitoring as `Unsupported`. Microphone gets a
/// real check via CPAL.
#[cfg(not(target_os = "macos"))]
#[test]
fn detect_permissions_non_macos_reports_unsupported_for_desktop_perms() {
    let status = detect_permissions();
    assert_eq!(
        status.screen_recording,
        PermissionState::Unsupported,
        "screen_recording should be Unsupported on non-macOS"
    );
    assert_eq!(
        status.accessibility,
        PermissionState::Unsupported,
        "accessibility should be Unsupported on non-macOS"
    );
    assert_eq!(
        status.input_monitoring,
        PermissionState::Unsupported,
        "input_monitoring should be Unsupported on non-macOS"
    );
}

/// Microphone permission on non-macOS/non-Windows platforms should be
/// `Granted` (standard Linux desktop) or `Unknown`/`Denied` (Flatpak). It
/// should never be `Unsupported` unless the platform has a dedicated stub.
#[cfg(target_os = "linux")]
#[test]
fn detect_microphone_permission_linux_returns_valid_state() {
    let state = detect_microphone_permission();
    assert!(
        !matches!(state, PermissionState::Unsupported),
        "Linux microphone state should not be Unsupported"
    );
}

// ── PermissionState serde round-trip ─────────────────────────────────────

#[test]
fn permission_state_serde_round_trip() {
    use crate::openhuman::accessibility::types::{PermissionState, PermissionStatus};

    let status = PermissionStatus {
        screen_recording: PermissionState::Granted,
        accessibility: PermissionState::Denied,
        input_monitoring: PermissionState::Unknown,
        microphone: PermissionState::Unsupported,
    };
    let json = serde_json::to_string(&status).expect("serialize PermissionStatus");
    let back: PermissionStatus = serde_json::from_str(&json).expect("deserialize PermissionStatus");
    assert_eq!(back.screen_recording, PermissionState::Granted);
    assert_eq!(back.accessibility, PermissionState::Denied);
    assert_eq!(back.input_monitoring, PermissionState::Unknown);
    assert_eq!(back.microphone, PermissionState::Unsupported);
}

// ── No stale denied cache across restart (automation_state) ───────────────
//
// The `automation_state` module exposes a process-local atomic flag. The
// "no stale denied cache across restart" guarantee is enforced by the fact
// that the flag is `AtomicBool` initialized to `false` — each new process
// starts clean. The tests below verify the clean-start invariant at the
// module level and that `clear()` restores the initial state, which is the
// mechanism used by `autocomplete::start_if_enabled` on re-engagement.

mod automation_state_stale_cache {
    use crate::openhuman::accessibility::{
        clear_automation_denial, mark_system_events_denied, system_events_denied,
    };
    use std::sync::Mutex;

    static LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn fresh_state_is_not_denied() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_automation_denial();
        assert!(
            !system_events_denied(),
            "after clear(), system_events_denied should be false (simulates process restart)"
        );
    }

    #[test]
    fn clear_resets_denial_flag() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_automation_denial();
        mark_system_events_denied();
        assert!(system_events_denied(), "should be denied after mark");
        clear_automation_denial();
        assert!(
            !system_events_denied(),
            "clear() should erase the stale denied state"
        );
    }

    #[test]
    fn denied_flag_does_not_persist_through_clear() {
        let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Simulate: previous session left the flag set.
        // clear() is called on re-engagement → no stale state carried over.
        mark_system_events_denied();
        clear_automation_denial();
        // Re-query after clear — must be false, simulating a "fresh sidecar" read.
        assert!(
            !system_events_denied(),
            "denial flag must not persist after clear() — no stale cache"
        );
    }
}
