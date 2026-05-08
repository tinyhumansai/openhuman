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

// SVG data URIs use URL encoding rather than base64 because:
//   1. base64-encoded `data:image/svg+xml` has tripped on strict
//      image-src CSPs in some Meet builds, manifesting as the bridge's
//      "mascot decode failed Event" warning with no further detail.
//   2. The SVGs already minify well; url-encoding only inflates the
//      reserved characters, while base64 inflates the whole payload by
//      33%. Net wire size is comparable.

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
/// data URIs. Cheap to compute and stable per process; the inject path
/// can memoize via `OnceLock` if it ever grows hot.
pub fn build_camera_bridge_js() -> String {
    let idle = svg_to_data_uri(MASCOT_IDLE_SVG);
    let thinking = svg_to_data_uri(MASCOT_THINKING_SVG);
    CAMERA_BRIDGE_TEMPLATE
        .replace("__OPENHUMAN_MASCOT_IDLE_DATAURI__", &idle)
        .replace("__OPENHUMAN_MASCOT_THINKING_DATAURI__", &thinking)
}

/// URL-encode an SVG into a `data:image/svg+xml` URI suitable for
/// `<img src>`. Conservative whitelist of unreserved characters per
/// RFC 3986 plus a few path-safe extras; everything else is
/// percent-encoded byte-by-byte (UTF-8). Earlier passes that escaped
/// only the obvious breakers (`<`, `>`, `"`, `#`, `%`) left raw spaces
/// in attribute values like `viewBox="0 0 1000 1000"`, which Chromium
/// rejects in data URIs (manifests as the bridge's
/// "mascot decode failed Event" warning with no further detail).
fn svg_to_data_uri(svg: &str) -> String {
    fn is_unreserved(b: u8) -> bool {
        matches!(b,
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~'
            // Sub-delims + path-safe that don't trip data-URI parsers
            // and keep the SVG body itself parseable. Notably: '/'
            // and ':' are fine inside path components per RFC 3986.
            | b'/' | b':' | b';' | b'=' | b',' | b'(' | b')'
            | b'*' | b'!' | b'\''
        )
    }

    let mut out = String::with_capacity(svg.len() * 2 + 64);
    out.push_str("data:image/svg+xml;charset=utf-8,");
    for byte in svg.bytes() {
        if is_unreserved(byte) {
            out.push(byte as char);
        } else {
            use std::fmt::Write as _;
            let _ = write!(out, "%{byte:02X}");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_substitutes_both_dataurus() {
        let js = build_camera_bridge_js();
        assert!(!js.contains("__OPENHUMAN_MASCOT_IDLE_DATAURI__"));
        assert!(!js.contains("__OPENHUMAN_MASCOT_THINKING_DATAURI__"));
        let count = js.matches("data:image/svg+xml;charset=utf-8,").count();
        assert!(count >= 2, "expected at least 2 data URIs, got {count}");
    }

    #[test]
    fn url_encoding_escapes_reserved_chars() {
        let uri = svg_to_data_uri("<svg width=\"10\"/>\n");
        assert!(uri.starts_with("data:image/svg+xml;charset=utf-8,"));
        let body = uri.trim_start_matches("data:image/svg+xml;charset=utf-8,");
        // The breakers — '<', '>', '"', '\n' — must not appear unescaped.
        assert!(!body.contains('<'));
        assert!(!body.contains('>'));
        assert!(!body.contains('"'));
        assert!(!body.contains('\n'));
        assert!(body.contains("%3C"));
        assert!(body.contains("%3E"));
        assert!(body.contains("%22"));
    }

    #[test]
    fn embedded_mascots_are_nonempty() {
        assert!(MASCOT_IDLE_SVG.len() > 100);
        assert!(MASCOT_THINKING_SVG.len() > 100);
        assert!(MASCOT_IDLE_SVG.contains("<svg"));
        assert!(MASCOT_THINKING_SVG.contains("<svg"));
    }
}
