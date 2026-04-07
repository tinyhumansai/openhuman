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
pub fn insert_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        warn!("{LOG_PREFIX} transcription was empty/whitespace, skipping insertion");
        return Ok(());
    }

    info!(
        "{LOG_PREFIX} inserting {} chars via clipboard-paste",
        text.len()
    );

    // Step 1: Save current clipboard.
    let mut clipboard =
        Clipboard::new().map_err(|e| format!("failed to access clipboard: {e}"))?;
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
        assert!(insert_text("").is_ok());
    }

    #[test]
    fn whitespace_only_skips_insertion() {
        assert!(insert_text("   ").is_ok());
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
