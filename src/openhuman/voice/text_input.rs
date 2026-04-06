//! Text insertion into the currently active text field.
//!
//! Uses enigo to simulate keyboard input so that transcribed text
//! appears in whatever application has focus.

use log::{debug, info, warn};

use enigo::{Enigo, Keyboard, Settings};

const LOG_PREFIX: &str = "[voice_input]";

/// Insert text into the currently active text field via enigo.
///
/// Skips empty or whitespace-only input. Uses enigo's `text()` method
/// which handles Unicode and platform-appropriate input simulation.
pub fn insert_text(text: &str) -> Result<(), String> {
    if text.trim().is_empty() {
        warn!("{LOG_PREFIX} transcription was empty/whitespace, skipping insertion");
        return Ok(());
    }

    info!(
        "{LOG_PREFIX} inserting {} chars into active field",
        text.len()
    );

    let mut enigo = Enigo::new(&Settings::default())
        .map_err(|e| format!("failed to create enigo instance: {e}"))?;

    enigo
        .text(text)
        .map_err(|e| format!("failed to insert text: {e}"))?;

    debug!("{LOG_PREFIX} text inserted successfully");
    Ok(())
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
}
