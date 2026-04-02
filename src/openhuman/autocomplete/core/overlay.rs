//! Overflow badge, overlay display, and macOS notifications.
//!
//! Overlay rendering is delegated to the shared `accessibility` middleware module.

#[cfg(target_os = "macos")]
use chrono::Utc;
#[cfg(target_os = "macos")]
use once_cell::sync::Lazy;
#[cfg(target_os = "macos")]
use std::sync::Mutex as StdMutex;

use super::text::truncate_tail;
use crate::openhuman::accessibility::{self, ElementBounds};

#[cfg(target_os = "macos")]
static LAST_OVERFLOW_BADGE: Lazy<StdMutex<Option<(String, i64)>>> =
    Lazy::new(|| StdMutex::new(None));

#[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
pub(super) fn show_overflow_badge(
    kind: &str,
    suggestion: Option<&str>,
    error: Option<&str>,
    app_name: Option<&str>,
    anchor_bounds: Option<&ElementBounds>,
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
                let fallback_bounds = ElementBounds {
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                };
                let bounds = if anchor_bounds.is_some() {
                    anchor_bounds.unwrap()
                } else {
                    log::debug!(
                        "[autocomplete] overlay: no anchor bounds, falling back to zero bounds (mouse cursor); suggestion={:?}",
                        truncate_tail(suggestion_text, 40)
                    );
                    &fallback_bounds
                };
                if accessibility::show_overlay(bounds, suggestion_text, 1100).is_ok() {
                    return;
                }
            }
        } else {
            let _ = accessibility::hide_overlay();
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

/// Quit the overlay helper process.
pub(super) fn overlay_helper_quit() -> Result<(), String> {
    accessibility::quit_overlay()
}
