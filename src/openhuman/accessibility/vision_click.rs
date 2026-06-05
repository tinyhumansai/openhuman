//! Vision fallback for `automate`: click a described on-screen element when an
//! app exposes no usable accessibility tree.
//!
//! Electron/Chromium apps (Slack, Discord, VS Code) expose little or no AX/UIA,
//! so the perceive→press loop has nothing to act on (tracker §1.5). The answer
//! is *screenshot → vision-locate → guarded click*:
//!
//!   1. **screenshot** the target app's window (`super::capture`), recording the
//!      window's screen rect so pixels can be mapped back to screen points.
//!   2. **locate** — ask the main vision model for the PIXEL coordinates of the
//!      described element, passing the image via the provider's `[IMAGE:<uri>]`
//!      marker (promoted to a real image part in
//!      `inference/provider/compatible_types.rs`, #3205).
//!   3. **map** image pixels → absolute screen points ([`image_to_screen`]).
//!   4. **guarded click** — only ever issued by the caller once the target app
//!      is frontmost, never into OpenHuman's own window (the §1.8 CEF-crash
//!      guard). Synthetic input runs on the app main thread via
//!      `run_input_on_main` (Change 1.15 — off-thread enigo traps TSM).
//!
//! The coordinate transform and the model-response parser are pure and unit
//! tested; the capture / model / click side-effects sit behind them.

use super::types::{AppContext, ElementBounds};

const LOG_PREFIX: &str = "[vision_click]";

/// Geometry needed to map a point in the (possibly downscaled) screenshot back
/// to absolute screen coordinates.
///
/// `rect_*` is the captured window's screen rect in **points** (from
/// [`AppContext::bounds`]); `img_*` is the screenshot's **pixel** size. The
/// pixel→point ratio absorbs both the capture downscale *and* the Retina backing
/// scale, so the mapping needs no explicit scale factor (this is what closes the
/// deferred F2 coordinate-mapping gap).
// `pub` (not `pub(crate)`) so it doesn't read as "more private" than the
// `pub AutomateBackend` trait methods that name it; the enclosing `mod
// vision_click` is private, so it stays crate-internal in practice.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaptureGeometry {
    pub rect_x: i32,
    pub rect_y: i32,
    pub rect_w_pts: i32,
    pub rect_h_pts: i32,
    pub img_w_px: u32,
    pub img_h_px: u32,
}

impl CaptureGeometry {
    fn from_bounds(b: &ElementBounds, img_w_px: u32, img_h_px: u32) -> Self {
        Self {
            rect_x: b.x,
            rect_y: b.y,
            rect_w_pts: b.width,
            rect_h_pts: b.height,
            img_w_px,
            img_h_px,
        }
    }
}

/// Map an image-pixel coordinate to an absolute screen point.
///
/// Clamps the input to the image and the output to the window rect, so a model
/// that returns an out-of-range guess still lands inside the *target* window —
/// never elsewhere on screen (and never on OpenHuman's own window).
pub(crate) fn image_to_screen(geom: &CaptureGeometry, px: i32, py: i32) -> (i32, i32) {
    let img_w = geom.img_w_px.max(1) as f64;
    let img_h = geom.img_h_px.max(1) as f64;
    // Clamp the sampled pixel into the image, then express it as a 0..1 fraction.
    let fx = (px.max(0) as f64).min(img_w - 1.0) / img_w;
    let fy = (py.max(0) as f64).min(img_h - 1.0) / img_h;
    let sx = geom.rect_x as f64 + fx * geom.rect_w_pts as f64;
    let sy = geom.rect_y as f64 + fy * geom.rect_h_pts as f64;
    // Keep the result strictly inside the window rect.
    let max_x = geom.rect_x + (geom.rect_w_pts - 1).max(0);
    let max_y = geom.rect_y + (geom.rect_h_pts - 1).max(0);
    (
        (sx.round() as i32).clamp(geom.rect_x, max_x),
        (sy.round() as i32).clamp(geom.rect_y, max_y),
    )
}

/// The JSON the vision model must return: whether the element was found and its
/// center pixel coordinates within the screenshot.
#[derive(Debug, serde::Deserialize)]
struct LocateResponse {
    #[serde(default)]
    found: bool,
    #[serde(default)]
    x: i32,
    #[serde(default)]
    y: i32,
}

/// System prompt pinning the locate contract for the vision model.
fn locate_system_prompt() -> &'static str {
    "You locate a single UI element in a screenshot of one application window. \
     You are given the image and a description of the element to click. Reply \
     with EXACTLY ONE JSON object and nothing else: \
     {\"found\":true,\"x\":<int>,\"y\":<int>} where x,y are the PIXEL \
     coordinates (origin top-left) of the CENTER of that element within the \
     image. If the element is not visible, reply {\"found\":false,\"x\":0,\"y\":0}. \
     Output JSON only — no prose, no code fences."
}

/// Build the user turn: the target description plus the screenshot as a
/// `[IMAGE:<data-uri>]` marker the compatible provider promotes to an image part.
fn build_locate_user(description: &str, screenshot_data_uri: &str) -> String {
    format!("Element to click: {description}\n[IMAGE:{screenshot_data_uri}]")
}

/// Parse the model's locate reply, tolerating code fences / surrounding prose by
/// extracting the first balanced `{...}` (mirrors `automate::parse_action`).
/// `found:false` → `Ok(None)`; unparseable → `Err` so the caller can report it
/// rather than act on a hallucinated guess (tracker §1.13 lesson).
fn parse_locate_response(raw: &str) -> Result<Option<(i32, i32)>, String> {
    let trimmed = raw.trim();
    let parsed = serde_json::from_str::<LocateResponse>(trimmed).or_else(|_| {
        match (trimmed.find('{'), trimmed.rfind('}')) {
            (Some(s), Some(e)) if e > s => serde_json::from_str::<LocateResponse>(&trimmed[s..=e]),
            // Re-run on the trimmed text so the error type matches the arm.
            _ => serde_json::from_str::<LocateResponse>(trimmed),
        }
    });
    match parsed {
        Ok(r) if r.found => Ok(Some((r.x, r.y))),
        Ok(_) => Ok(None),
        Err(_) => Err(format!("could not parse locate response: {trimmed:?}")),
    }
}

/// Decode the pixel dimensions of a `data:image/...;base64,...` URI.
fn image_dims_from_data_uri(data_uri: &str) -> Result<(u32, u32), String> {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    use image::GenericImageView as _;
    let b64 = data_uri
        .split_once(',')
        .map(|(_, payload)| payload)
        .ok_or_else(|| "malformed image data URI (no base64 payload)".to_string())?;
    let bytes = BASE64_STANDARD
        .decode(b64)
        .map_err(|e| format!("screenshot base64 decode failed: {e}"))?;
    let img =
        image::load_from_memory(&bytes).map_err(|e| format!("screenshot decode failed: {e}"))?;
    Ok(img.dimensions())
}

/// Capture the app window and pair it with the geometry needed to map a click.
///
/// Requires `ctx.bounds` (the window's screen rect) — without it the image
/// pixels can't be mapped to screen points, so we refuse rather than click blind.
pub(crate) fn capture_window_geometry(
    ctx: &AppContext,
) -> Result<(String, CaptureGeometry), String> {
    let bounds = ctx
        .bounds
        .ok_or_else(|| "window bounds unavailable — cannot map click coordinates".to_string())?;
    let data_uri = super::capture::capture_screen_image_ref_for_context(Some(ctx))?;
    let (w, h) = image_dims_from_data_uri(&data_uri)?;
    log::debug!(
        "{LOG_PREFIX} captured window rect=({},{},{},{}) image={}x{}px",
        bounds.x,
        bounds.y,
        bounds.width,
        bounds.height,
        w,
        h
    );
    Ok((data_uri, CaptureGeometry::from_bounds(&bounds, w, h)))
}

/// Ask the vision model for the target's pixel coordinates within `screenshot`.
/// Returns `Ok(None)` when the model reports the element isn't visible.
pub(crate) async fn locate_via_vision(
    provider: &dyn crate::openhuman::inference::provider::Provider,
    model: &str,
    screenshot_data_uri: &str,
    description: &str,
) -> Result<Option<(i32, i32)>, String> {
    let user = build_locate_user(description, screenshot_data_uri);
    let raw = provider
        .chat_with_system(Some(locate_system_prompt()), &user, model, 0.0)
        .await
        .map_err(|e| format!("vision model call failed: {e}"))?;
    log::debug!("{LOG_PREFIX} locate raw response: {raw:?}");
    parse_locate_response(&raw)
}

/// Single guarded left-click at absolute screen coordinates, run on the app
/// main thread. Off-thread enigo traps macOS TSM and crashes the CEF host
/// (Change 1.15 / §1.8) — so the click closure is dispatched via
/// `run_input_on_main`, exactly like the `mouse` tool.
pub(crate) async fn guarded_click(x: i32, y: i32) -> Result<String, String> {
    use crate::openhuman::tools::implementations::run_input_on_main;
    log::info!("{LOG_PREFIX} ▶ click at screen ({x}, {y})");
    run_input_on_main(move || {
        use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
        let mut enigo =
            Enigo::new(&Settings::default()).map_err(|e| format!("enigo init failed: {e}"))?;
        enigo
            .move_mouse(x, y, Coordinate::Abs)
            .map_err(|e| format!("move_mouse failed: {e}"))?;
        enigo
            .button(Button::Left, Direction::Click)
            .map_err(|e| format!("click failed: {e}"))?;
        Ok(format!("Clicked at ({x}, {y})"))
    })
    .await
}

#[cfg(test)]
#[path = "vision_click_tests.rs"]
mod tests;
