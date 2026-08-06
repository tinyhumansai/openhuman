//! Platform accessibility middleware: focus queries and permission management.
//!
//! Centralises macOS AX/IOKit FFI and the unified Swift helper process.
//! Voice services call into this module instead of owning platform-specific
//! code directly.

mod automation_state;
mod focus;
mod globe;
mod helper;
mod permissions;
mod terminal;
mod text_util;
mod types;

pub use automation_state::{
    clear as clear_automation_denial, mark_system_events_denied, system_events_denied,
};
pub use focus::{focused_text_context, focused_text_context_verbose, validate_focused_target};
pub use globe::{
    globe_listener_poll, globe_listener_start, globe_listener_stop, GlobeHotkeyPollResult,
    GlobeHotkeyStatus,
};
pub use helper::precompile_helper_background;
#[cfg(target_os = "macos")]
pub use permissions::{
    detect_accessibility_permission, detect_input_monitoring_permission, open_macos_privacy_pane,
    request_accessibility_access,
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
    ElementBounds, FocusedTextContext, PermissionKind, PermissionState, PermissionStatus,
};
