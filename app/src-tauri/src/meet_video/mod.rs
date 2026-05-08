//! Meet camera bridge — overrides the agent webview's outbound video
//! track with a programmatically rendered mascot.
//!
//! ## Why JS injection (not a CEF patch)
//!
//! The `--use-file-for-fake-video-capture` flag we already pass at
//! browser startup (see [`crate::fake_camera`]) reads a single Y4M and
//! loops at EOF, which produces a static image. The flag is process-
//! level; we cannot rebind it per-call without rebuilding Chromium
//! from source. Until we own that build pipeline, the only way to
//! produce a *dynamic* outbound camera is to intercept
//! `navigator.mediaDevices.getUserMedia` at the JS layer and substitute
//! a `MediaStream` from a `<canvas>` we own.
//!
//! This is a deliberate, scoped exception to the "no JS injection into
//! CEF child webviews" rule (see CLAUDE.md). Google Meet is already on
//! the grandfathered list (`audio_bridge.js`, `captions_bridge.js`),
//! and the camera bridge follows the same install path.
//!
//! ## Pieces
//!
//! - [`camera_bridge.js`] (sibling file, embedded via `include_str!`):
//!   page-side bridge. Builds a hidden 640×480 canvas, decodes the
//!   idle + thinking mascot SVGs, runs an rAF loop, exposes the
//!   resulting `canvas.captureStream(30)` through monkey-patched
//!   `getUserMedia` + `enumerateDevices`. Carries an unconditional 5s
//!   mood toggle as the default driver; the host can also call
//!   `window.__openhumanSetMood(name)` over CDP at any time.
//!
//! - [`inject`] — installs the bridge via CDP
//!   `Page.addScriptToEvaluateOnNewDocument`. Wired into
//!   [`crate::meet_audio::inject::install_audio_bridge`] so a single
//!   `Page.reload` boots all three bridges (audio + captions + camera).
//!
//! - This file — embeds the two mascot SVGs at build time and templates
//!   them into the bridge JS as `data:image/svg+xml;base64,...` URIs,
//!   keeping the bridge fully self-contained inside the Meet origin.

pub mod inject;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;

/// Idle mascot SVG (calm, eyes-forward). Rasterized into the canvas
/// during the bridge's `ready` promise.
const MASCOT_IDLE_SVG: &str = include_str!("../../../../remotion/public/idelMascot.svg");

/// Thinking mascot SVG (book-reading pose) — toggled in/out as the
/// agent's "thinking" state. Picked over `Cupholding`/`syicsmile` for
/// the most legible mood difference; revisit when phase 2 swaps the
/// static SVG for a live Remotion-driven OSR feed.
const MASCOT_THINKING_SVG: &str = include_str!("../../../../remotion/public/Bookreading.svg");

/// Bridge JS template. Two `__OPENHUMAN_MASCOT_*_DATAURI__` tokens are
/// substituted at install time with base64'd SVG data URIs.
const CAMERA_BRIDGE_TEMPLATE: &str = include_str!("camera_bridge.js");

/// Build the page-side camera bridge JS with the mascot SVGs inlined as
/// data URIs. Done at runtime (not `const`) because `base64::encode` is
/// not const, but the result is cheap to compute and stable per process,
/// so the inject path memoizes it via `OnceLock` if it grows hot.
pub fn build_camera_bridge_js() -> String {
    let idle = svg_to_data_uri(MASCOT_IDLE_SVG);
    let thinking = svg_to_data_uri(MASCOT_THINKING_SVG);
    CAMERA_BRIDGE_TEMPLATE
        .replace("__OPENHUMAN_MASCOT_IDLE_DATAURI__", &idle)
        .replace("__OPENHUMAN_MASCOT_THINKING_DATAURI__", &thinking)
}

fn svg_to_data_uri(svg: &str) -> String {
    let b64 = BASE64.encode(svg.as_bytes());
    format!("data:image/svg+xml;base64,{b64}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_substitutes_both_dataurus() {
        let js = build_camera_bridge_js();
        assert!(!js.contains("__OPENHUMAN_MASCOT_IDLE_DATAURI__"));
        assert!(!js.contains("__OPENHUMAN_MASCOT_THINKING_DATAURI__"));
        assert!(js.contains("data:image/svg+xml;base64,"));
        // Both mascots should appear (two distinct data URIs).
        let count = js.matches("data:image/svg+xml;base64,").count();
        assert!(count >= 2, "expected at least 2 data URIs, got {count}");
    }

    #[test]
    fn data_uri_round_trips() {
        let uri = svg_to_data_uri("<svg/>");
        assert!(uri.starts_with("data:image/svg+xml;base64,"));
        let b64 = uri.trim_start_matches("data:image/svg+xml;base64,");
        let decoded = BASE64.decode(b64).expect("base64 decodes");
        assert_eq!(decoded, b"<svg/>");
    }

    #[test]
    fn embedded_mascots_are_nonempty() {
        assert!(MASCOT_IDLE_SVG.len() > 100);
        assert!(MASCOT_THINKING_SVG.len() > 100);
        assert!(MASCOT_IDLE_SVG.contains("<svg"));
        assert!(MASCOT_THINKING_SVG.contains("<svg"));
    }
}
