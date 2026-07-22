//! Platform accessibility middleware: focus queries, text insertion, key state,
//! overlays, screen capture, and permission management.
//!
//! Centralises all macOS AX/CGEvent/IOKit FFI and the unified Swift helper process.
//! Consumer modules (autocomplete, screen_intelligence, voice) call into this module
//! instead of owning platform-specific code directly.
//!
//! Facade for the `desktop-automation` gate (#5049): the FFI-bearing submodules are
//! `#[cfg(feature = "desktop-automation")]`; the inert `types` module (and the
//! `GlobeHotkey*` structs it now owns) stay compiled in both directions so the
//! always-on callers (`text_input`, `voice`, `app_state`, …) keep the one real type
//! definition. When the feature is off, `stub` re-exposes the public *behaviour*
//! surface those callers reach, with disabled-error / no-op / denied bodies.

// Inert, dependency-free platform types — compiled in BOTH builds (type carve-out).
// The `GlobeHotkeyStatus` / `GlobeHotkeyPollResult` structs live here too (moved out
// of the FFI `globe` module, which re-exports them) so they resolve with the feature
// off. See AGENTS.md "skills gate — the type carve-out".
pub mod types;

#[cfg(feature = "desktop-automation")]
pub mod app_fastpaths;
#[cfg(feature = "desktop-automation")]
pub mod automate;
#[cfg(feature = "desktop-automation")]
mod automation_state;
#[cfg(feature = "desktop-automation")]
pub mod ax_interact;
#[cfg(feature = "desktop-automation")]
mod capture;
// Pure ranked/normalized matching of listed AX elements against a target label
// — the "reliable UI element clicking" selection primitive (no FFI). Consumed by
// `ax_interact.rs` to order filtered results best-first.
#[cfg(feature = "desktop-automation")]
mod element_match;
#[cfg(feature = "desktop-automation")]
mod focus;
#[cfg(feature = "desktop-automation")]
mod globe;
#[cfg(feature = "desktop-automation")]
mod helper;
#[cfg(feature = "desktop-automation")]
mod keys;
#[cfg(feature = "desktop-automation")]
mod overlay;
#[cfg(feature = "desktop-automation")]
mod paste;
#[cfg(feature = "desktop-automation")]
mod permissions;
#[cfg(feature = "desktop-automation")]
mod terminal;
#[cfg(feature = "desktop-automation")]
mod text_util;
// Vision fallback for `automate`: screenshot → vision-locate → guarded click,
// for Electron/partial-AX apps. Consumed by `automate.rs`'s `RealBackend`.
#[cfg(feature = "desktop-automation")]
mod vision_click;
// Windows accessibility backend for `ax_interact` (UI Automation). Sibling of
// the macOS Swift-helper path; selected via cfg-dispatch in `ax_interact.rs`.
#[cfg(all(feature = "desktop-automation", target_os = "windows"))]
mod uia_interact;

#[cfg(not(feature = "desktop-automation"))]
mod stub;
#[cfg(not(feature = "desktop-automation"))]
pub use stub::*;

#[cfg(feature = "desktop-automation")]
pub use automation_state::{
    clear as clear_automation_denial, mark_system_events_denied, system_events_denied,
};
#[cfg(feature = "desktop-automation")]
pub use capture::{capture_screen_image_ref_for_context, CaptureMode, MAX_SCREENSHOT_BYTES};
#[cfg(feature = "desktop-automation")]
pub use element_match::{best_match, ElementMatch, MatchTier};
#[cfg(feature = "desktop-automation")]
pub use focus::{
    focused_text_context, focused_text_context_verbose, foreground_context,
    parse_foreground_output, validate_focused_target,
};
#[cfg(feature = "desktop-automation")]
pub use globe::{globe_listener_poll, globe_listener_start, globe_listener_stop};
#[cfg(feature = "desktop-automation")]
pub use helper::precompile_helper_background;
#[cfg(feature = "desktop-automation")]
pub use keys::{any_modifier_down, is_escape_key_down, is_tab_key_down};
#[cfg(feature = "desktop-automation")]
pub use overlay::{hide_overlay, quit_overlay, show_overlay};
#[cfg(feature = "desktop-automation")]
pub use paste::{apply_text_to_focused_field, send_backspace};
#[cfg(all(feature = "desktop-automation", target_os = "macos"))]
pub use permissions::{
    detect_accessibility_permission, detect_input_monitoring_permission,
    detect_screen_recording_permission, open_macos_privacy_pane, request_accessibility_access,
    request_screen_recording_access,
};
#[cfg(feature = "desktop-automation")]
pub use permissions::{
    detect_microphone_permission, detect_permissions, microphone_denied_message, permission_to_str,
    request_microphone_access,
};
#[cfg(feature = "desktop-automation")]
pub use terminal::{
    extract_terminal_input_context, is_terminal_app, is_text_role, looks_like_terminal_buffer,
};
#[cfg(feature = "desktop-automation")]
pub use text_util::{normalize_ax_value, parse_ax_number, truncate_tail};

// Carved types — compiled in BOTH builds. The `GlobeHotkey*` structs moved out of
// the FFI `globe` module into `types`; re-export them here (ungated) so
// `screen_intelligence::types` and other carved consumers resolve in both builds.
// With the feature ON, `globe` also re-exports them, so the enabled build keeps
// its historical `accessibility::globe::GlobeHotkeyStatus` path too.
pub use types::{
    AppContext, ElementBounds, FocusedTextContext, GlobeHotkeyPollResult, GlobeHotkeyStatus,
    PermissionKind, PermissionState, PermissionStatus,
};
