//! Accessibility focus, clipboard/paste insertion, and key state probes.

use super::terminal::{is_terminal_app, is_text_role};
use super::text::{normalize_ax_value, parse_ax_number, truncate_tail};
use super::types::{FocusedElementBounds, FocusedTextContext};

#[cfg(target_os = "macos")]
pub(super) fn focused_text_context() -> Result<FocusedTextContext, String> {
    let ctx = focused_text_context_verbose()?;
    if let Some(err) = ctx.raw_error.as_ref() {
        return Err(format!(
            "focused text unavailable via accessibility api: {err}"
        ));
    }
    Ok(ctx)
}

#[cfg(target_os = "macos")]
pub(super) fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    let script = r##"
      tell application "System Events"
        set sep to character id 31
        set frontApp to first application process whose frontmost is true
        set appName to name of frontApp
        set roleValue to "unknown"
        set textValue to ""
        set selectedValue to ""
        set errValue to ""
        set posX to ""
        set posY to ""
        set sizeW to ""
        set sizeH to ""
        set targetRoles to {"AXTextArea", "AXTextField", "AXSearchField", "AXComboBox", "AXEditableText"}

        -- Enable AXEnhancedUserInterface for Chromium-based apps (Chrome, Electron, VS Code, Slack, etc.)
        -- Without this, these apps do not properly expose focused text elements via Accessibility API.
        try
          set value of attribute "AXEnhancedUserInterface" of frontApp to true
        end try

        try
          set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
          try
            set roleValue to value of attribute "AXRole" of focusedElement as text
          end try
          try
            set textValue to value of attribute "AXValue" of focusedElement as text
          end try
          try
            set p to value of attribute "AXPosition" of focusedElement
            set posX to item 1 of p as text
            set posY to item 2 of p as text
          end try
          try
            set s to value of attribute "AXSize" of focusedElement
            set sizeW to item 1 of s as text
            set sizeH to item 2 of s as text
          end try
          if textValue is "missing value" then set textValue to ""
          if textValue is "" then
            try
              set selectedValue to value of attribute "AXSelectedText" of focusedElement as text
            end try
            if selectedValue is "missing value" then set selectedValue to ""
            if selectedValue is not "" then set textValue to selectedValue
          end if
          if textValue is "" then
            try
              set textValue to value of attribute "AXTitle" of focusedElement as text
            end try
            if textValue is "missing value" then set textValue to ""
          end if
        on error errMsg number errNum
          set errValue to "ERROR:" & errNum & ":" & errMsg
        end try

        if textValue is "" then
          try
            set focusedWindow to value of attribute "AXFocusedWindow" of frontApp
            set childElems to entire contents of focusedWindow
            set staticPromptValue to ""
            set staticFallbackValue to ""
            repeat with childElem in childElems
              set childRole to ""
              set childValue to ""
              set childSelectedValue to ""
              try
                set childRole to value of attribute "AXRole" of childElem as text
              end try
              if childRole is in targetRoles then
                try
                  set childValue to value of attribute "AXValue" of childElem as text
                end try
                set childPosX to ""
                set childPosY to ""
                set childSizeW to ""
                set childSizeH to ""
                try
                  set cp to value of attribute "AXPosition" of childElem
                  set childPosX to item 1 of cp as text
                  set childPosY to item 2 of cp as text
                end try
                try
                  set cs to value of attribute "AXSize" of childElem
                  set childSizeW to item 1 of cs as text
                  set childSizeH to item 2 of cs as text
                end try
                if childValue is "missing value" then set childValue to ""
                if childValue is "" then
                  try
                    set childSelectedValue to value of attribute "AXSelectedText" of childElem as text
                  end try
                  if childSelectedValue is "missing value" then set childSelectedValue to ""
                  if childSelectedValue is not "" then set childValue to childSelectedValue
                end if
                if childValue is not "" then
                  set roleValue to childRole
                  set textValue to childValue
                  if childPosX is not "" then set posX to childPosX
                  if childPosY is not "" then set posY to childPosY
                  if childSizeW is not "" then set sizeW to childSizeW
                  if childSizeH is not "" then set sizeH to childSizeH
                  exit repeat
                end if
              end if
            end repeat
            if textValue is "" then
              repeat with childElem in childElems
                set childRole to ""
                set childValue to ""
                try
                  set childRole to value of attribute "AXRole" of childElem as text
                end try
                if childRole is "AXStaticText" then
                  try
                    set childValue to value of attribute "AXValue" of childElem as text
                  end try
                  if childValue is "missing value" then set childValue to ""
                  if childValue is not "" then
                    set staticFallbackValue to childValue
                    if childValue contains "$ " or childValue contains "# " or childValue contains "> " then
                      set staticPromptValue to childValue
                    end if
                  end if
                end if
              end repeat
              if staticPromptValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticPromptValue
              else if staticFallbackValue is not "" then
                set roleValue to "AXStaticText"
                set textValue to staticFallbackValue
              end if
            end if
          on error errMsg2 number errNum2
            if errValue is "" then set errValue to "ERROR:" & errNum2 & ":" & errMsg2
          end try
        end if

        if textValue is "" and errValue is "" then
          set errValue to "ERROR:no_text_candidate_found"
        end if

        return appName & sep & roleValue & sep & textValue & sep & selectedValue & sep & errValue & sep & posX & sep & posY & sep & sizeW & sep & sizeH
      end tell
    "##;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("unable to query focused text context".to_string());
        }
        return Err(format!("unable to query focused text context: {stderr}"));
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let trimmed = text.trim_end_matches(['\r', '\n']);
    let mut segments = trimmed.splitn(9, '\u{1f}');
    let app_name = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let role = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let mut value = segments.next().map(normalize_ax_value).unwrap_or_default();
    let mut selected_text = segments
        .next()
        .map(normalize_ax_value)
        .filter(|s| !s.is_empty());
    let mut raw_error = segments
        .next()
        .map(|s| normalize_ax_value(s.trim()))
        .filter(|s| !s.is_empty());
    let pos_x = segments.next().and_then(parse_ax_number);
    let pos_y = segments.next().and_then(parse_ax_number);
    let size_w = segments.next().and_then(parse_ax_number);
    let size_h = segments.next().and_then(parse_ax_number);

    let allow_terminal_text_value =
        is_terminal_app(app_name.as_deref()) && !value.trim().is_empty();
    if !is_text_role(role.as_deref()) && !allow_terminal_text_value {
        value.clear();
        selected_text = None;
        if raw_error.is_none() {
            raw_error = Some("ERROR:no_text_candidate_found".to_string());
        }
    }

    Ok(FocusedTextContext {
        app_name,
        role,
        text: value,
        selected_text,
        raw_error,
        bounds: match (pos_x, pos_y, size_w, size_h) {
            (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
                Some(FocusedElementBounds {
                    x,
                    y,
                    width,
                    height,
                })
            }
            _ => None,
        },
    })
}

#[cfg(not(target_os = "macos"))]
pub(super) fn focused_text_context() -> Result<FocusedTextContext, String> {
    Err("autocomplete is only supported on macOS".to_string())
}

#[cfg(not(target_os = "macos"))]
pub(super) fn focused_text_context_verbose() -> Result<FocusedTextContext, String> {
    Err("autocomplete is only supported on macOS".to_string())
}

fn normalize_ax_value(raw: &str) -> String {
    let v = raw.trim();
    if v.eq_ignore_ascii_case("missing value") {
        String::new()
    } else {
        v.to_string()
    }
}

fn parse_ax_number(raw: &str) -> Option<i32> {
    let trimmed = normalize_ax_value(raw);
    if trimmed.is_empty() {
        return None;
    }
    let cleaned = trimmed.replace(',', ".");
    cleaned.parse::<f64>().ok().map(|v| v.round() as i32)
}

/// Validate that the currently focused element still matches the target we generated the
/// suggestion for. Returns Ok if it matches or if validation is inconclusive (to avoid
/// false negatives). Returns Err if it clearly does not match.
#[cfg(target_os = "macos")]
pub(super) fn validate_focused_target(
    expected_app: Option<&str>,
    expected_role: Option<&str>,
) -> Result<(), String> {
    if expected_app.is_none() {
        return Ok(()); // No target to validate against
    }
    let current = focused_text_context_verbose();
    match current {
        Ok(ctx) => {
            if let (Some(expected), Some(actual)) = (expected_app, ctx.app_name.as_deref()) {
                if expected.to_lowercase() != actual.to_lowercase() {
                    return Err(format!(
                        "focus shifted from '{}' to '{}', aborting insertion",
                        expected, actual
                    ));
                }
            }
            // Role check is advisory — some apps change role dynamically
            if let (Some(expected), Some(actual)) = (expected_role, ctx.role.as_deref()) {
                if expected != actual {
                    log::debug!(
                        "[autocomplete] target role changed from '{}' to '{}', proceeding anyway",
                        expected,
                        actual
                    );
                }
            }
            Ok(())
        }
        Err(_) => Ok(()), // Validation inconclusive, proceed
    }
}

/// Save the current clipboard contents, returning the text (or None if non-text/empty).
#[cfg(target_os = "macos")]
fn clipboard_save() -> Option<String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg("the clipboard as text")
        .output()
        .ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout).trim_end().to_string();
        if text.is_empty() || text == "missing value" {
            None
        } else {
            Some(text)
        }
    } else {
        None
    }
}

/// Set the system clipboard to the given text.
#[cfg(target_os = "macos")]
fn clipboard_set(text: &str) -> Result<(), String> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', "\\n");
    let script = format!(r#"set the clipboard to "{}""#, escaped);
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to set clipboard: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("failed to set clipboard: {stderr}"));
    }
    Ok(())
}

/// Simulate Cmd+V keypress via CGEvent to paste clipboard contents into the focused field.
/// This works universally across all apps (unlike direct AXValue writes).
#[cfg(target_os = "macos")]
fn simulate_paste() {
    unsafe {
        let key_down = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_V, true);
        let key_up = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_V, false);
        if key_down.is_null() || key_up.is_null() {
            log::warn!("[autocomplete] failed to create CGEvent for paste");
            return;
        }
        CGEventSetFlags(key_down, KCG_EVENT_FLAG_MASK_COMMAND);
        CGEventSetFlags(key_up, KCG_EVENT_FLAG_MASK_COMMAND);
        CGEventPost(KCG_HID_EVENT_TAP, key_down);
        std::thread::sleep(std::time::Duration::from_millis(8));
        CGEventPost(KCG_HID_EVENT_TAP, key_up);
    }
}

/// Primary insertion method: clipboard + simulated Cmd+V paste.
/// Saves and restores the original clipboard contents.
#[cfg(target_os = "macos")]
fn paste_text_via_clipboard(text: &str) -> Result<(), String> {
    let original_clipboard = clipboard_save();
    clipboard_set(text)?;

    // Brief delay to ensure clipboard is set before paste
    std::thread::sleep(std::time::Duration::from_millis(10));
    simulate_paste();

    // Restore original clipboard after a delay so the paste has time to complete
    if let Some(original) = original_clipboard {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let _ = clipboard_set(&original);
        });
    }

    Ok(())
}

/// Fallback insertion method: direct AXValue write via AppleScript.
/// Works well for simple text editors but fails on many web/Electron inputs.
#[cfg(target_os = "macos")]
fn apply_text_via_axvalue(text: &str) -> Result<(), String> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', " ");
    let script = format!(
        r##"
tell application "System Events"
  set frontApp to first application process whose frontmost is true
  set focusedElement to value of attribute "AXFocusedUIElement" of frontApp
  set currentValue to ""
  try
    set currentValue to value of attribute "AXValue" of focusedElement as text
  end try
  if currentValue is "missing value" then set currentValue to ""
  if currentValue is "" then
    try
      set currentValue to value of attribute "AXSelectedText" of focusedElement as text
    end try
    if currentValue is "missing value" then set currentValue to ""
  end if
  set value of attribute "AXValue" of focusedElement to (currentValue & "{}")
end tell
"##,
        escaped
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("failed to apply suggestion to focused text field".to_string());
        }
        return Err(format!(
            "failed to apply suggestion to focused text field: {stderr}"
        ));
    }
    Ok(())
}

/// Apply suggestion text to the focused field.
/// Uses clipboard+paste (reliable across all apps) as the primary method,
/// with direct AXValue write as fallback.
#[cfg(target_os = "macos")]
pub(super) fn apply_text_to_focused_field(text: &str) -> Result<(), String> {
    log::debug!(
        "[autocomplete] applying text via clipboard+paste: {:?}",
        truncate_tail(text, 40)
    );
    match paste_text_via_clipboard(text) {
        Ok(()) => Ok(()),
        Err(paste_err) => {
            log::warn!(
                "[autocomplete] clipboard+paste failed ({}), falling back to AXValue write",
                paste_err
            );
            apply_text_via_axvalue(text)
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn apply_text_to_focused_field(_text: &str) -> Result<(), String> {
    Err("autocomplete is only supported on macOS".to_string())
}

#[cfg(target_os = "macos")]
pub(super) fn is_tab_key_down() -> bool {
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_TAB) }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn is_tab_key_down() -> bool {
    false
}

#[cfg(target_os = "macos")]
pub(super) fn is_escape_key_down() -> bool {
    unsafe { CGEventSourceKeyState(KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE, KVK_ESCAPE) }
}

#[cfg(not(target_os = "macos"))]
pub(super) fn is_escape_key_down() -> bool {
    false
}

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
extern "C" {
    fn CGEventSourceKeyState(state_id: i32, key: u16) -> bool;
}

#[cfg(target_os = "macos")]
#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[cfg(target_os = "macos")]
#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(
        source: *const std::ffi::c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> *mut std::ffi::c_void;
    fn CGEventSetFlags(event: *mut std::ffi::c_void, flags: u64);
    fn CGEventPost(tap: u32, event: *mut std::ffi::c_void);
}

#[cfg(target_os = "macos")]
const KCG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE: i32 = 0;
#[cfg(target_os = "macos")]
const KVK_TAB: u16 = 48;
#[cfg(target_os = "macos")]
const KVK_ESCAPE: u16 = 53;
#[cfg(target_os = "macos")]
const KVK_V: u16 = 9;
#[cfg(target_os = "macos")]
const KCG_HID_EVENT_TAP: u32 = 0;
#[cfg(target_os = "macos")]
const KCG_EVENT_FLAG_MASK_COMMAND: u64 = 0x00100000;
