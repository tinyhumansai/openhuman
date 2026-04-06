//! Text insertion into focused fields via accessibility APIs.
//!
//! Three-tier strategy: (1) Swift helper paste, (2) osascript clipboard + CGEvent, (3) AXValue write.

use super::text_util::truncate_tail;

/// Apply suggestion text to the focused field.
/// Tries: (1) helper paste, (2) osascript clipboard+CGEvent, (3) AXValue write.
#[cfg(target_os = "macos")]
pub fn apply_text_to_focused_field(text: &str) -> Result<(), String> {
    log::debug!(
        "[accessibility] applying text: {:?}",
        truncate_tail(text, 40)
    );

    // Try 1: unified Swift helper (handles clipboard save/set/paste/restore internally)
    match paste_text_via_helper(text) {
        Ok(()) => return Ok(()),
        Err(e) => {
            log::debug!(
                "[accessibility] helper paste failed ({}), trying osascript+CGEvent",
                e
            );
        }
    }

    // Try 2: osascript clipboard + CGEvent Cmd+V
    match paste_text_via_osascript_cgevent(text) {
        Ok(()) => return Ok(()),
        Err(e) => {
            log::debug!(
                "[accessibility] osascript+CGEvent paste failed ({}), trying AXValue write",
                e
            );
        }
    }

    // Try 3: direct AXValue write (last resort)
    apply_text_via_axvalue(text)
}

/// Paste via the unified Swift helper.
#[cfg(target_os = "macos")]
fn paste_text_via_helper(text: &str) -> Result<(), String> {
    let request = serde_json::json!({"type": "paste", "text": text});
    let resp = super::helper::helper_send_receive(&request)?;
    let ok = resp.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    if ok {
        Ok(())
    } else {
        let err = resp
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown paste error");
        Err(err.to_string())
    }
}

/// Paste via osascript (clipboard set) + CGEvent (Cmd+V simulation).
#[cfg(target_os = "macos")]
fn paste_text_via_osascript_cgevent(text: &str) -> Result<(), String> {
    let original_clipboard = clipboard_save_osascript();

    // Set clipboard via osascript — preserve multi-line text using AppleScript linefeed.
    let script = {
        let lines: Vec<String> = text
            .split('\n')
            .map(|line| {
                let escaped = line.replace('\\', "\\\\").replace('\"', "\\\"");
                format!("\"{}\"", escaped)
            })
            .collect();
        let joined = lines.join(" & linefeed & ");
        format!("set the clipboard to ({})", joined)
    };
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to set clipboard: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(format!("failed to set clipboard: {stderr}"));
    }

    std::thread::sleep(std::time::Duration::from_millis(10));

    // Cmd+V via CGEvent
    unsafe {
        let key_down = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_V, true);
        let key_up = CGEventCreateKeyboardEvent(std::ptr::null(), KVK_V, false);
        if key_down.is_null() || key_up.is_null() {
            if !key_down.is_null() {
                CFRelease(key_down as *const _);
            }
            if !key_up.is_null() {
                CFRelease(key_up as *const _);
            }
            return Err("failed to create CGEvent for paste".to_string());
        }
        CGEventSetFlags(key_down, KCG_EVENT_FLAG_MASK_COMMAND);
        CGEventSetFlags(key_up, KCG_EVENT_FLAG_MASK_COMMAND);
        CGEventPost(KCG_HID_EVENT_TAP, key_down);
        std::thread::sleep(std::time::Duration::from_millis(8));
        CGEventPost(KCG_HID_EVENT_TAP, key_up);
        CFRelease(key_down as *const _);
        CFRelease(key_up as *const _);
    }

    // Restore clipboard
    if let Some(original) = original_clipboard {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let lines: Vec<String> = original
                .split('\n')
                .map(|line| {
                    let escaped = line.replace('\\', "\\\\").replace('\"', "\\\"");
                    format!("\"{}\"", escaped)
                })
                .collect();
            let joined = lines.join(" & linefeed & ");
            let script = format!("set the clipboard to ({})", joined);
            let _ = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output();
        });
    }

    Ok(())
}

#[cfg(target_os = "macos")]
fn clipboard_save_osascript() -> Option<String> {
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg("the clipboard as text")
        .output()
        .ok()?;
    if output.status.success() {
        let text = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        if text.is_empty() || text == "missing value" {
            None
        } else {
            Some(text)
        }
    } else {
        None
    }
}

/// Fallback insertion: direct AXValue write via AppleScript.
/// Reads `AXSelectedTextRange` to insert at the cursor position rather than
/// always appending to the end of the field.
#[cfg(target_os = "macos")]
fn apply_text_via_axvalue(text: &str) -> Result<(), String> {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace('\n', " ");
    // AXSelectedTextRange.location is 0-based; AppleScript string indices are 1-based.
    // "text 1 thru 0" evaluates to "" in AppleScript — correct for cursor-at-start.
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
  -- Read cursor position from AXSelectedTextRange (0-based location).
  set insertionOffset to -1
  try
    set selRange to value of attribute "AXSelectedTextRange" of focusedElement
    set insertionOffset to location of selRange
  end try
  -- Insert at cursor when available, otherwise append.
  if insertionOffset >= 0 and insertionOffset <= (length of currentValue) then
    if insertionOffset = 0 then
      set value of attribute "AXValue" of focusedElement to ("{}" & currentValue)
    else if insertionOffset >= (length of currentValue) then
      set value of attribute "AXValue" of focusedElement to (currentValue & "{}")
    else
      set beforeCursor to text 1 thru insertionOffset of currentValue
      set afterCursor to text (insertionOffset + 1) thru -1 of currentValue
      set value of attribute "AXValue" of focusedElement to (beforeCursor & "{}" & afterCursor)
    end if
  else
    set value of attribute "AXValue" of focusedElement to (currentValue & "{}")
  end if
end tell
"##,
        escaped, escaped, escaped, escaped
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            return Err("failed to apply text to focused field".to_string());
        }
        return Err(format!("failed to apply text to focused field: {stderr}"));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn apply_text_to_focused_field(_text: &str) -> Result<(), String> {
    Err("text insertion is only supported on macOS".to_string())
}

// ---------------------------------------------------------------------------
// macOS FFI declarations for paste
// ---------------------------------------------------------------------------

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
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[cfg(target_os = "macos")]
const KVK_V: u16 = 9;
#[cfg(target_os = "macos")]
const KCG_HID_EVENT_TAP: u32 = 0;
#[cfg(target_os = "macos")]
const KCG_EVENT_FLAG_MASK_COMMAND: u64 = 0x00100000;
