//! Disabled-build stub for the `accessibility` domain (`desktop-automation` off).
//!
//! Reproduces the public *behaviour* surface that always-compiled / other-gated
//! callers reach — `text_input`, `voice`, `app_state`, `screen_intelligence`
//! (itself gated) — with disabled-error / no-op / denied bodies. The inert types
//! stay in `types.rs` and are re-exported by the facade, so there is zero type
//! duplication here (the carve-out from AGENTS.md).
//!
//! Signatures match the real ones byte-for-byte; the slim build
//! (`--no-default-features --features tokenjuice-treesitter`) is the only thing
//! that catches drift between this file and the real module.

use super::types::{
    AppContext, ElementBounds, FocusedTextContext, GlobeHotkeyPollResult, GlobeHotkeyStatus,
    PermissionKind, PermissionState, PermissionStatus,
};

const DISABLED: &str =
    "desktop automation is disabled in this build (rebuild with --features desktop-automation)";

// ── focus ────────────────────────────────────────────────────────────────

/// Real: `focus::validate_focused_target`. The macOS arm inspects the AX tree;
/// the non-macOS arm returns `Ok(())`. With automation compiled out there is no
/// focus to validate, so treat it as inconclusive-pass (matching the non-macOS
/// real behaviour).
pub fn validate_focused_target(
    _expected_app: Option<&str>,
    _expected_role: Option<&str>,
) -> Result<(), String> {
    Ok(())
}

/// Real: `focus::focused_text_context_verbose`. No AX backend when disabled.
pub fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    Err(DISABLED.to_string())
}

/// Real: `focus::focused_text_context`.
pub fn focused_text_context() -> Result<FocusedTextContext, String> {
    Err(DISABLED.to_string())
}

/// Real: `focus::foreground_context`.
pub fn foreground_context() -> Option<AppContext> {
    None
}

// ── terminal ─────────────────────────────────────────────────────────────

/// Real: `terminal::is_terminal_app`. Pure heuristic in the real module, but it
/// lives inside the gated tree; without automation there is no focused app, so
/// nothing is a terminal.
pub fn is_terminal_app(_app_name: Option<&str>) -> bool {
    false
}

// ── paste ────────────────────────────────────────────────────────────────

/// Real: `paste::apply_text_to_focused_field`.
pub fn apply_text_to_focused_field(_text: &str) -> Result<(), String> {
    // Grep-friendly: never logs the text itself (PII).
    log::debug!("[accessibility] apply_text_to_focused_field disabled: desktop-automation off");
    Err(DISABLED.to_string())
}

/// Real: `paste::send_backspace`.
pub fn send_backspace(_count: usize) -> Result<(), String> {
    log::debug!("[accessibility] send_backspace disabled: desktop-automation off");
    Err(DISABLED.to_string())
}

// ── overlay ──────────────────────────────────────────────────────────────

/// Real: `overlay::show_overlay`. The non-macOS real arm is a no-op `Ok(())`;
/// match it — with no overlay helper there is nothing to draw.
pub fn show_overlay(
    _bounds: &ElementBounds,
    _text: &str,
    _ttl_ms: u32,
    _tab_hint: &str,
) -> Result<(), String> {
    Ok(())
}

/// Real: `overlay::hide_overlay`.
pub fn hide_overlay() -> Result<(), String> {
    Ok(())
}

/// Real: `overlay::quit_overlay`.
pub fn quit_overlay() -> Result<(), String> {
    Ok(())
}

// ── globe ────────────────────────────────────────────────────────────────

/// Real: `globe::globe_listener_start`. Re-exports the carved `GlobeHotkey*`
/// types; the listener helper is compiled out, so report unsupported.
pub fn globe_listener_start() -> Result<GlobeHotkeyStatus, String> {
    log::debug!("[accessibility] globe_listener_start disabled: desktop-automation off");
    Ok(disabled_globe_status())
}

/// Real: `globe::globe_listener_poll`.
pub fn globe_listener_poll() -> Result<GlobeHotkeyPollResult, String> {
    Ok(GlobeHotkeyPollResult {
        status: disabled_globe_status(),
        events: Vec::new(),
    })
}

/// Real: `globe::globe_listener_stop`.
pub fn globe_listener_stop() -> Result<GlobeHotkeyStatus, String> {
    Ok(disabled_globe_status())
}

fn disabled_globe_status() -> GlobeHotkeyStatus {
    GlobeHotkeyStatus {
        supported: false,
        running: false,
        input_monitoring_permission: PermissionState::Unsupported,
        last_error: Some(DISABLED.to_string()),
        events_pending: 0,
    }
}

// ── permissions ──────────────────────────────────────────────────────────

/// Real: `permissions::detect_microphone_permission`. Microphone capture lives in
/// the `voice` domain, which reads its permission through here — so this stub is
/// load-bearing when `voice` is enabled but `desktop-automation` is not.
///
/// The real Linux / non-macOS implementation cpal-probes and returns `Granted`
/// when an input device is available; the voice recorder treats `Unknown` as
/// "request then re-check, else fail", so returning `Unknown` here would break
/// dictation before it ever opens the mic. Match the permissive non-desktop arm
/// (`Granted`) so voice proceeds and any real capture error surfaces from cpal.
pub fn detect_microphone_permission() -> PermissionState {
    PermissionState::Granted
}

/// Real: `permissions::request_microphone_access`. No-op when disabled.
pub fn request_microphone_access() {}

/// Real: `permissions::microphone_denied_message`.
pub fn microphone_denied_message() -> String {
    "Microphone permission could not be determined in this build.".to_string()
}

/// Real: `permissions::permission_to_str`.
pub fn permission_to_str(permission: PermissionKind) -> &'static str {
    match permission {
        PermissionKind::ScreenRecording => "screen_recording",
        PermissionKind::Accessibility => "accessibility",
        PermissionKind::InputMonitoring => "input_monitoring",
        PermissionKind::Microphone => "microphone",
    }
}

/// Real: `permissions::detect_permissions`. The desktop-automation permissions
/// (screen-recording / accessibility / input-monitoring) are `Unsupported` when
/// the gate is off — matching the real non-macOS `detect_permissions`, which
/// reports these as `Unsupported` so the disabled build "behaves like a
/// non-desktop platform". Microphone mirrors `detect_microphone_permission`
/// above so the `voice`-on / `desktop-automation`-off build keeps a usable mic
/// state.
pub fn detect_permissions() -> PermissionStatus {
    PermissionStatus {
        screen_recording: PermissionState::Unsupported,
        accessibility: PermissionState::Unsupported,
        input_monitoring: PermissionState::Unsupported,
        microphone: PermissionState::Granted,
    }
}

// ── automation state ─────────────────────────────────────────────────────

/// Real: `automation_state::mark_system_events_denied`. No shared denial state
/// exists when automation is compiled out, so these are no-ops / `false`.
pub fn mark_system_events_denied() {}

/// Real: `automation_state::clear` (re-exported as `clear_automation_denial`).
pub fn clear_automation_denial() {}

/// Real: `automation_state::system_events_denied`.
pub fn system_events_denied() -> bool {
    false
}

// ── automate ─────────────────────────────────────────────────────────────

/// Disabled-build mirror of `accessibility::automate`. Reached by
/// `voice::always_on::execute_intent`, which builds a `RealBackend`, calls `run`,
/// and reads `.success` / `.summary` off the returned outcome.
pub mod automate {
    use crate::openhuman::config::Config;

    use super::DISABLED;

    /// Real: `automate::AutomateOutcome`.
    #[derive(Debug, Clone, PartialEq)]
    pub struct AutomateOutcome {
        pub success: bool,
        pub summary: String,
        pub steps: Vec<String>,
    }

    /// Real: `automate::AutomateOptions`. The real module hand-writes `Default`
    /// (a non-zero `DEFAULT_STEP_BUDGET`); the disabled path never steps, so the
    /// derived zero-default is correct here.
    #[derive(Debug, Clone, Copy, Default)]
    pub struct AutomateOptions {
        pub step_budget: u32,
    }

    /// Real: `automate::RealBackend`. The config is retained to match the real
    /// constructor signature exactly, even though the disabled path never uses it.
    pub struct RealBackend {
        #[allow(dead_code)]
        config: Config,
    }

    impl RealBackend {
        pub fn new(config: Config) -> Self {
            Self { config }
        }
    }

    /// Real: `automate::run`. The disabled build has no backend to drive, so it
    /// reports a failed outcome rather than performing any UI automation. The
    /// generic `backend` parameter keeps the caller's `&RealBackend` argument
    /// compiling without stubbing the `AutomateBackend` trait.
    pub async fn run<B>(
        _app: &str,
        _goal: &str,
        _backend: &B,
        _opts: AutomateOptions,
    ) -> AutomateOutcome {
        // Grep-friendly: never logs `_app` / `_goal` (may carry user content).
        log::debug!("[accessibility] automate::run disabled: desktop-automation off");
        AutomateOutcome {
            success: false,
            summary: DISABLED.to_string(),
            steps: Vec::new(),
        }
    }
}
