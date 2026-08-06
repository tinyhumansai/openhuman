# Accessibility

Cross-platform accessibility middleware. Owns macOS AX / CGEvent / IOKit FFI, the unified Swift helper-process bridge, focused-text inspection, system-permission detection (Accessibility, Input Monitoring, Microphone), the Globe-key listener, the floating overlay window, paste / backspace key synthesis, terminal heuristics, and AX-string normalization. Centralises platform-specific code so that `voice` never touches FFI directly.

## Public surface

- `pub fn focused_text_context` / `focused_text_context_verbose` / `validate_focused_target` — `focus.rs` — query the OS for the currently focused text field.
- `pub fn globe_listener_start` / `globe_listener_stop` / `globe_listener_poll` / `pub struct GlobeHotkeyPollResult` / `pub enum GlobeHotkeyStatus` — `globe.rs` — macOS Globe-key (Fn) hotkey monitor.
- `pub fn precompile_helper_background` — `helper.rs` — warm the Swift helper process at startup.
- `pub fn any_modifier_down` / `is_escape_key_down` / `is_tab_key_down` — `keys.rs` — modifier polling for cancellation gestures.
- `pub fn show_overlay` / `hide_overlay` / `quit_overlay` — `overlay.rs` — floating completion overlay control.
- `pub fn apply_text_to_focused_field` / `pub fn send_backspace` — `paste.rs` — programmatic text insertion.
- Permission detection: `detect_permissions`, `detect_microphone_permission`, `microphone_denied_message`, `permission_to_str`, `request_microphone_access` (cross-platform); macOS-only `detect_accessibility_permission`, `detect_input_monitoring_permission`, `open_macos_privacy_pane`, `request_accessibility_access` — `permissions.rs`.
- `pub fn extract_terminal_input_context` / `is_terminal_app` / `is_text_role` / `looks_like_terminal_buffer` — `terminal.rs` — terminal-window heuristics.
- `pub fn normalize_ax_value` / `parse_ax_number` / `truncate_tail` — `text_util.rs` — AX value normalization.
- `pub struct ElementBounds` / `FocusedTextContext` / `PermissionKind` / `PermissionState` / `PermissionStatus` — `types.rs`.

## Calls into

- macOS frameworks (`ApplicationServices`, `CoreGraphics`, `IOKit`, `AVFoundation`) via FFI.
- Bundled Swift helper process for AX queries that require a separate process.
- `src/openhuman/config/` — overlay sizing and helper paths (light dependency).

## Called by

- `src/openhuman/voice/` — microphone permission and focused-text helpers (indirect, via re-exports).

## Tests

- Permission and focus coverage runs through `permissions_tests.rs`, inline module tests, and retained consumers.
- AX FFI surface is best validated end-to-end on a real macOS host — most CI runs are Linux and skip platform-gated paths.
