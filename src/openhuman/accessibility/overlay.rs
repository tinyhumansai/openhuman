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
