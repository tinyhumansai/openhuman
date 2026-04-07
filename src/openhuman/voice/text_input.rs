//! Text insertion into the currently active text field.
//!
//! Uses the **clipboard-paste** strategy (like OpenWhispr): writes text
//! to the system clipboard then simulates Cmd+V / Ctrl+V to paste it.
//! This is atomic and instantaneous, unlike enigo's `text()` which types
//! character-by-character and causes garbled/repeated output on macOS.
//!
//! The previous clipboard contents are saved and restored after a short
//! delay so the user's clipboard is not permanently overwritten.

use log::{debug, info, warn};
use std::time::Duration;

#[cfg(target_os = "macos")]
use crate::openhuman::accessibility;
use arboard::Clipboard;
use enigo::{Direction, Enigo, Key, Keyboard, Settings};

const LOG_PREFIX: &str = "[voice_input]";

/// Delay before sending Cmd+V, letting the clipboard write settle.
/// OpenWhispr uses 120ms on macOS.
const PASTE_DELAY: Duration = Duration::from_millis(120);

/// Delay after sending Cmd+V before restoring the clipboard, giving the
/// target application time to read from the clipboard.
/// OpenWhispr uses 450ms on macOS.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(450);
#[cfg(target_os = "macos")]
const FOCUS_RESTORE_DELAY: Duration = Duration::from_millis(100);

/// Insert text into the currently active text field via clipboard-paste.
///
/// Strategy:
/// 1. Save current clipboard contents
/// 2. Write transcribed text to clipboard
/// 3. Simulate Cmd+V (macOS) or Ctrl+V (Windows/Linux)
/// 4. Wait briefly, then restore original clipboard
///
/// This avoids the character-by-character typing issues with enigo's
/// `text()` method which causes garbled/repeated output.
pub fn insert_text(text: &str, expected_app: Option<&str>) -> Result<(), String> {
    if text.trim().is_empty() {
        warn!("{LOG_PREFIX} transcription was empty/whitespace, skipping insertion");
        return Ok(());
    }

    #[cfg(not(target_os = "macos"))]
    let _ = expected_app;

    info!(
        "{LOG_PREFIX} inserting {} chars via clipboard-paste",
        text.len()
    );

    // Step 1: Save current clipboard.
    let mut clipboard = Clipboard::new().map_err(|e| format!("failed to access clipboard: {e}"))?;
    let saved_clipboard = clipboard.get_text().ok();
    debug!(
        "{LOG_PREFIX} saved clipboard ({} chars)",
        saved_clipboard.as_ref().map_or(0, |s| s.len())
    );

    // Step 2: Write transcription to clipboard.
    clipboard
        .set_text(text)
        .map_err(|e| format!("failed to write text to clipboard: {e}"))?;
    debug!("{LOG_PREFIX} transcription written to clipboard");

    // Step 3: Brief delay to let clipboard write settle, then simulate paste.
    std::thread::sleep(PASTE_DELAY);

    #[cfg(target_os = "macos")]
    if let Some(app_name) = expected_app {
        debug!("{LOG_PREFIX} validating focus before paste; expected_app='{app_name}'");
        if let Err(validation_err) = accessibility::validate_focused_target(Some(app_name), None) {
            warn!("{LOG_PREFIX} focus changed before paste: {validation_err}");
            // Always try to restore focus — even if the user hasn't clicked a
            // text field yet, activating the app brings it to front and most
            // apps will accept Cmd+V into their last-focused element.
            if let Err(restore_err) = restore_focus_to_app(app_name) {
                warn!(
                    "{LOG_PREFIX} focus restore failed: {restore_err} — will attempt paste anyway"
                );
            } else {
                info!("{LOG_PREFIX} focus restored to '{app_name}' before paste");
            }
        }
    }

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("failed to create enigo instance: {e}"))?;

    let modifier = paste_modifier_key();
    enigo
        .key(modifier, Direction::Press)
        .map_err(|e| format!("failed to press modifier: {e}"))?;
    enigo
        .key(Key::Unicode('v'), Direction::Click)
        .map_err(|e| format!("failed to press 'v': {e}"))?;
    enigo
        .key(modifier, Direction::Release)
        .map_err(|e| format!("failed to release modifier: {e}"))?;

    debug!("{LOG_PREFIX} paste keystroke sent");

    // Step 4: Restore clipboard after a delay (non-blocking).
    if let Some(original) = saved_clipboard {
        std::thread::spawn(move || {
            std::thread::sleep(CLIPBOARD_RESTORE_DELAY);
            match Clipboard::new() {
                Ok(mut cb) => {
                    if let Err(e) = cb.set_text(&original) {
                        warn!("{LOG_PREFIX} failed to restore clipboard: {e}");
                    } else {
                        debug!("{LOG_PREFIX} clipboard restored");
                    }
                }
                Err(e) => warn!("{LOG_PREFIX} failed to re-open clipboard for restore: {e}"),
            }
        });
    }

    info!("{LOG_PREFIX} text inserted successfully via paste");
    Ok(())
}

#[cfg(target_os = "macos")]
fn restore_focus_to_app(app_name: &str) -> Result<(), String> {
    let script = format!(
        r#"tell application "{}" to activate"#,
        escape_applescript_string(app_name)
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .map_err(|e| format!("failed to run osascript for focus restore: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "unknown osascript error".to_string()
        } else {
            stderr
        };
        return Err(format!(
            "failed to restore focus to '{}': {}",
            app_name, detail
        ));
    }
    std::thread::sleep(FOCUS_RESTORE_DELAY);
    Ok(())
}

#[cfg(target_os = "macos")]
fn escape_applescript_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Returns the platform-appropriate paste modifier key.
fn paste_modifier_key() -> Key {
    if cfg!(target_os = "macos") {
        Key::Meta
    } else {
        Key::Control
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_text_is_noop() {
        assert!(insert_text("", None).is_ok());
    }

    #[test]
    fn whitespace_only_skips_insertion() {
        assert!(insert_text("   ", None).is_ok());
    }

    #[test]
    fn paste_modifier_is_platform_correct() {
        let key = paste_modifier_key();
        if cfg!(target_os = "macos") {
            assert!(matches!(key, Key::Meta));
        } else {
            assert!(matches!(key, Key::Control));
        }
    }
}
