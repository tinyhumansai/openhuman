//! Overlay display via the unified Swift helper process.

use super::text_util::truncate_tail;
use super::types::ElementBounds;

/// Show an overlay badge near the given element bounds.
#[cfg(target_os = "macos")]
pub fn show_overlay(bounds: &ElementBounds, text: &str, ttl_ms: u32) -> Result<(), String> {
    let message = serde_json::json!({
        "type": "show",
        "x": bounds.x,
        "y": bounds.y,
        "w": bounds.width,
        "h": bounds.height,
        "text": truncate_tail(text, 96),
        "ttl_ms": ttl_ms
    });
    super::helper::helper_send_fire_and_forget(&message)
}

/// Hide the overlay badge.
#[cfg(target_os = "macos")]
pub fn hide_overlay() -> Result<(), String> {
    let message = serde_json::json!({"type": "hide"});
    super::helper::helper_send_fire_and_forget(&message)
}

/// Quit the unified helper process (cleanup on shutdown).
#[cfg(target_os = "macos")]
pub fn quit_overlay() -> Result<(), String> {
    super::helper::helper_quit()
}

#[cfg(not(target_os = "macos"))]
pub fn show_overlay(_bounds: &ElementBounds, _text: &str, _ttl_ms: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn hide_overlay() -> Result<(), String> {
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub fn quit_overlay() -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(not(target_os = "macos"))]
    use super::*;

    // --- Non-macOS stubs always succeed ---

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overlay_non_macos_returns_ok() {
        let bounds = ElementBounds { x: 0, y: 0, width: 100, height: 50 };
        assert!(show_overlay(&bounds, "suggestion text", 900).is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overlay_non_macos_empty_text_returns_ok() {
        let bounds = ElementBounds { x: 10, y: 20, width: 0, height: 0 };
        assert!(show_overlay(&bounds, "", 500).is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overlay_non_macos_zero_ttl_returns_ok() {
        let bounds = ElementBounds { x: 0, y: 0, width: 200, height: 30 };
        assert!(show_overlay(&bounds, "hello", 0).is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overlay_non_macos_max_ttl_returns_ok() {
        let bounds = ElementBounds { x: -10, y: -5, width: 300, height: 60 };
        assert!(show_overlay(&bounds, "test", u32::MAX).is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn hide_overlay_non_macos_returns_ok() {
        assert!(hide_overlay().is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn quit_overlay_non_macos_returns_ok() {
        assert!(quit_overlay().is_ok());
    }

    // Verify overlay functions can be called multiple times without error
    #[cfg(not(target_os = "macos"))]
    #[test]
    fn hide_overlay_idempotent() {
        assert!(hide_overlay().is_ok());
        assert!(hide_overlay().is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn quit_overlay_idempotent() {
        assert!(quit_overlay().is_ok());
        assert!(quit_overlay().is_ok());
    }
}
