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
    ttl_ms: u32,
    // When `kind == "ready"`, show the Tab hint in the overlay only if true.
    show_tab_hint: bool,
) {
    #[cfg(target_os = "macos")]
    {
        let now_ms = Utc::now().timestamp_millis();
        let signature = format!(
            "{}:{}:{}:{}",
            kind,
            app_name.unwrap_or_default(),
            suggestion.unwrap_or_default(),
            error.unwrap_or_default()
        );

        // Deduplicate rapid duplicate events only (same payload within a short window).
        const DEDUP_MS: i64 = 400;
        if let Ok(mut guard) = LAST_OVERFLOW_BADGE.lock() {
            if let Some((last_signature, last_ms)) = guard.as_ref() {
                if *last_signature == signature && now_ms - *last_ms < DEDUP_MS {
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
                let tab_hint = if show_tab_hint { "Tab ↵" } else { "" };
                if accessibility::show_overlay(bounds, suggestion_text, ttl_ms, tab_hint).is_ok() {
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- overlay_helper_quit (cross-platform) ---

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn overlay_helper_quit_non_macos_returns_ok() {
        assert!(overlay_helper_quit().is_ok());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn overlay_helper_quit_non_macos_idempotent() {
        assert!(overlay_helper_quit().is_ok());
        assert!(overlay_helper_quit().is_ok());
    }

    // --- escape_osascript_text (macOS-only) ---

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_plain_string_unchanged() {
        assert_eq!(escape_osascript_text("hello world"), "hello world");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_escapes_double_quotes() {
        assert_eq!(escape_osascript_text(r#"say "hello""#), r#"say \"hello\""#);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_escapes_backslash() {
        assert_eq!(escape_osascript_text(r"back\slash"), r"back\\slash");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_replaces_newline_with_space() {
        assert_eq!(escape_osascript_text("line1\nline2"), "line1 line2");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_replaces_carriage_return_with_space() {
        assert_eq!(escape_osascript_text("line1\rline2"), "line1 line2");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_crlf_both_replaced() {
        // \r and \n are each replaced individually → two spaces
        assert_eq!(escape_osascript_text("a\r\nb"), "a  b");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_empty_string_unchanged() {
        assert_eq!(escape_osascript_text(""), "");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_backslash_before_quote_double_escapes() {
        // r#"\"# + `"` = `\"` — backslash first becomes `\\`, then `"` becomes `\"`
        assert_eq!(escape_osascript_text("\\\""), "\\\\\\\"");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn escape_osascript_text_multiple_quotes() {
        assert_eq!(
            escape_osascript_text(r#""a" and "b""#),
            r#"\"a\" and \"b\""#
        );
    }

    // --- show_overflow_badge signature (non-macOS no-op smoke test) ---

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overflow_badge_non_macos_does_not_panic_ready() {
        let bounds = crate::openhuman::accessibility::ElementBounds {
            x: 0,
            y: 0,
            width: 100,
            height: 50,
        };
        // Should be a no-op and not panic.
        show_overflow_badge(
            "ready",
            Some("suggestion"),
            None,
            Some("TestApp"),
            Some(&bounds),
            900,
            true,
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overflow_badge_non_macos_does_not_panic_error() {
        show_overflow_badge(
            "error",
            None,
            Some("something failed"),
            None,
            None,
            500,
            false,
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overflow_badge_non_macos_does_not_panic_accepted() {
        show_overflow_badge(
            "accepted",
            Some("accepted text"),
            None,
            None,
            None,
            0,
            false,
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn show_overflow_badge_non_macos_does_not_panic_rejected() {
        show_overflow_badge("rejected", None, None, None, None, 200, false);
    }
}
