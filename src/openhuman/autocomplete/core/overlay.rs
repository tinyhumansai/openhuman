//! Overflow badge, overlay display, and macOS notifications.
//!
//! Overlay rendering is delegated to the unified Swift helper process (helper.rs).

#[cfg(target_os = "macos")]
use chrono::Utc;
#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
#[cfg(target_os = "macos")]
use std::sync::Mutex as StdMutex;

use super::text::truncate_tail;
use super::types::FocusedElementBounds;

#[cfg(target_os = "macos")]
static LAST_OVERFLOW_BADGE: Lazy<StdMutex<Option<(String, i64)>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub(super) fn show_overflow_badge(
    kind: &str,
    suggestion: Option<&str>,
    error: Option<&str>,
    app_name: Option<&str>,
    anchor_bounds: Option<&FocusedElementBounds>,
) {
    #[cfg(target_os = "macos")]
    {
        const READY_THROTTLE_MS: i64 = 1_200;
        let now_ms = Utc::now().timestamp_millis();
        let signature = format!(
            "{}:{}:{}:{}",
            kind,
            app_name.unwrap_or_default(),
            suggestion.unwrap_or_default(),
            error.unwrap_or_default()
        );

        if let Ok(mut guard) = LAST_OVERFLOW_BADGE.lock() {
            if let Some((last_signature, last_ms)) = guard.as_ref() {
                if *last_signature == signature {
                    return;
                }
                if kind == "ready" && (now_ms - *last_ms) < READY_THROTTLE_MS {
                    return;
                }
            }
            *guard = Some((signature, now_ms));
        }

        if kind == "ready" {
            if let Some(suggestion_text) = suggestion {
                // Use anchor bounds if available, otherwise pass zero bounds
                // (the unified helper will fall back to mouse cursor position).
                let fallback_bounds = FocusedElementBounds {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                };
                let bounds = anchor_bounds.unwrap_or(&fallback_bounds);
                if overlay_helper_show(bounds, suggestion_text).is_ok() {
                    return;
                }
            }
        } else {
            let _ = overlay_helper_hide();
        }

        // Notification fallback when overlay helper fails
        let title = match kind {
            "ready" => "OpenHuman suggestion",
            "accepted" => "OpenHuman applied",
            "rejected" => "OpenHuman dismissed",
            "error" => "OpenHuman autocomplete error",
            _ => "OpenHuman autocomplete",
        };

        let mut body = match kind {
            "ready" => suggestion.unwrap_or_default().to_string(),
            "accepted" => format!("Inserted: {}", suggestion.unwrap_or_default()),
            "rejected" => "Suggestion dismissed.".to_string(),
            "error" => error.unwrap_or("Autocomplete failed").to_string(),
            _ => suggestion.unwrap_or_default().to_string(),
        };
        if body.trim().is_empty() {
            body = "No suggestion".to_string();
        }
        body = truncate_tail(&body, 140);

        let subtitle = app_name.unwrap_or_default().trim().to_string();
        let escaped_title = escape_osascript_text(title);
        let escaped_body = escape_osascript_text(&body);
        let escaped_subtitle = escape_osascript_text(&subtitle);

        let script = if subtitle.is_empty() {
            format!(
                r#"display notification "{}" with title "{}""#,
                escaped_body, escaped_title
            )
        } else {
            format!(
                r#"display notification "{}" with title "{}" subtitle "{}""#,
                escaped_body, escaped_title, escaped_subtitle
            )
        };

        std::thread::spawn(move || {
            let _ = std::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output();
        });
    }
}

#[cfg(target_os = "macos")]
fn escape_osascript_text(raw: &str) -> String {
    raw.replace('\\', "\\\\")
        .replace('\"', "\\\"")
        .replace(['\n', '\r'], " ")
}

/// Show overlay via the unified Swift helper.
#[cfg(target_os = "macos")]
fn overlay_helper_show(bounds: &FocusedElementBounds, text: &str) -> Result<(), String> {
    let message = serde_json::json!({
        "type": "show",
        "x": bounds.x,
        "y": bounds.y,
        "w": bounds.width,
        "h": bounds.height,
        "text": truncate_tail(text, 96),
        "ttl_ms": 1100
    });
    super::helper::helper_send_fire_and_forget(&message)
}

/// Hide overlay via the unified Swift helper.
#[cfg(target_os = "macos")]
fn overlay_helper_hide() -> Result<(), String> {
    let message = serde_json::json!({"type": "hide"});
    super::helper::helper_send_fire_and_forget(&message)
}

/// Quit the unified helper process.
#[cfg(target_os = "macos")]
pub(super) fn overlay_helper_quit() -> Result<(), String> {
    super::helper::helper_quit()
}

#[cfg(not(target_os = "macos"))]
pub(super) fn overlay_helper_quit() -> Result<(), String> {
    Ok(())
}
