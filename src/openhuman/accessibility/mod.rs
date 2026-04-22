//! Platform accessibility middleware: focus queries, text insertion, key state,
//! overlays, screen capture, and permission management.
//!
//! Centralises all macOS AX/CGEvent/IOKit FFI and the unified Swift helper process.
//! Consumer modules (autocomplete, screen_intelligence, voice) call into this module
//! instead of owning platform-specific code directly.

mod capture;
mod focus;
mod globe;
mod helper;
mod keys;
mod overlay;
mod paste;
mod permissions;
mod terminal;
mod text_util;
mod types;

pub use capture::{capture_screen_image_ref_for_context, CaptureMode, MAX_SCREENSHOT_BYTES};
pub use focus::{
    focused_text_context, focused_text_context_verbose, foreground_context,
    parse_foreground_output, validate_focused_target,
};
pub use globe::{
    globe_listener_poll, globe_listener_start, globe_listener_stop, GlobeHotkeyPollResult,
    GlobeHotkeyStatus,
};
pub use helper::precompile_helper_background;
pub use keys::{any_modifier_down, is_escape_key_down, is_tab_key_down};
pub use overlay::{hide_overlay, quit_overlay, show_overlay};
pub use paste::{apply_text_to_focused_field, send_backspace};
#[cfg(target_os = "macos")]
pub use permissions::{
    detect_accessibility_permission, detect_input_monitoring_permission,
    detect_screen_recording_permission, open_macos_privacy_pane, request_accessibility_access,
    request_screen_recording_access,
};
pub use permissions::{
    detect_microphone_permission, detect_permissions, microphone_denied_message, permission_to_str,
    request_microphone_access,
};
pub use terminal::{
    extract_terminal_input_context, is_terminal_app, is_text_role, looks_like_terminal_buffer,
};
pub use text_util::{normalize_ax_value, parse_ax_number, truncate_tail};
pub use types::{
    AppContext, ElementBounds, FocusedTextContext, PermissionKind, PermissionState,
    PermissionStatus,
};
