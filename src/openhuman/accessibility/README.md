# Accessibility

Cross-platform accessibility middleware. Owns macOS AX / CGEvent / IOKit FFI, the unified Swift helper-process bridge, screen capture, focused-text + foreground app inspection, system-permission detection (Accessibility, Input Monitoring, Screen Recording, Microphone), the Globe-key listener, the floating overlay window, paste / backspace key synthesis, terminal heuristics, and AX-string normalization. Centralises platform-specific code so that `autocomplete`, `screen_intelligence`, and `voice` never touch FFI directly.

## Public surface

- `pub fn capture_screen_image_ref_for_context` / `pub enum CaptureMode` / `pub const MAX_SCREENSHOT_BYTES` — `capture.rs` — bounded screen capture.
- `pub fn focused_text_context` / `focused_text_context_verbose` / `foreground_context` / `parse_foreground_output` / `validate_focused_target` — `focus.rs` — query the OS for the currently focused text field and frontmost app.
- `pub fn globe_listener_start` / `globe_listener_stop` / `globe_listener_poll` / `pub struct GlobeHotkeyPollResult` / `pub enum GlobeHotkeyStatus` — `globe.rs` — macOS Globe-key (Fn) hotkey monitor.
- `pub fn precompile_helper_background` — `helper.rs` — warm the Swift helper process at startup.
- `pub fn any_modifier_down` / `is_escape_key_down` / `is_tab_key_down` — `keys.rs` — modifier polling for cancellation gestures.
- `pub fn show_overlay` / `hide_overlay` / `quit_overlay` — `overlay.rs` — floating completion overlay control.
- `pub fn apply_text_to_focused_field` / `pub fn send_backspace` — `paste.rs` — programmatic text insertion.
- Permission detection: `detect_permissions`, `detect_microphone_permission`, `microphone_denied_message`, `permission_to_str`, `request_microphone_access` (cross-platform); macOS-only `detect_accessibility_permission`, `detect_input_monitoring_permission`, `detect_screen_recording_permission`, `open_macos_privacy_pane`, `request_accessibility_access`, `request_screen_recording_access` — `permissions.rs`.
- `pub fn extract_terminal_input_context` / `is_terminal_app` / `is_text_role` / `looks_like_terminal_buffer` — `terminal.rs` — terminal-window heuristics.
- `pub fn normalize_ax_value` / `parse_ax_number` / `truncate_tail` — `text_util.rs` — AX value normalization.
- `pub struct AppContext` / `ElementBounds` / `FocusedTextContext` / `PermissionKind` / `PermissionState` / `PermissionStatus` — `types.rs`.

## Calls into

- macOS frameworks (`ApplicationServices`, `CoreGraphics`, `IOKit`, `AVFoundation`) via FFI.
- Bundled Swift helper process for AX queries that require a separate process.
- `src/openhuman/config/` — overlay sizing and helper paths (light dependency).

## Called by

- `src/openhuman/autocomplete/core/{terminal,text,overlay,types,focus}.rs` — focus-driven autocomplete needs every accessibility primitive.
- `src/openhuman/screen_intelligence/{types,state,capture_worker,input,tests}.rs` — screen capture + focus context for vision pipelines.
- `src/openhuman/voice/` — microphone permission + foreground app context (indirect, via re-exports).
- `src/core/` — surfaces `AccessibilityStatus` snapshots for the shell.

## Tests

- This domain has no `*_tests.rs` siblings; coverage runs through the consumer modules' `tests.rs` (notably `screen_intelligence/tests.rs`) and integration tests in `tests/screen_intelligence_vision_e2e.rs`.
- AX FFI surface is best validated end-to-end on a real macOS host — most CI runs are Linux and skip platform-gated paths.
