//! Unit tests for the vision-click primitives. The coordinate transform and the
//! locate-response parser are pure, so they're exercised with no screen, no
//! model, and no synthetic input.

use super::*;

fn geom(
    rect_x: i32,
    rect_y: i32,
    rect_w_pts: i32,
    rect_h_pts: i32,
    img_w_px: u32,
    img_h_px: u32,
) -> CaptureGeometry {
    CaptureGeometry {
        rect_x,
        rect_y,
        rect_w_pts,
        rect_h_pts,
        img_w_px,
        img_h_px,
    }
}

// ── image_to_screen ─────────────────────────────────────────────────────────

#[test]
fn maps_center_when_image_matches_rect() {
    // Image pixels == window points (no scaling): center maps to center.
    let g = geom(0, 0, 1000, 800, 1000, 800);
    assert_eq!(image_to_screen(&g, 500, 400), (500, 400));
}

#[test]
fn maps_through_downscaled_screenshot() {
    // Capture was downscaled to half size; a center pixel still maps to the
    // window center in screen points.
    let g = geom(0, 0, 1000, 800, 500, 400);
    assert_eq!(image_to_screen(&g, 250, 200), (500, 400));
}

#[test]
fn maps_through_retina_2x_backing_scale() {
    // Retina capture is 2× the window's point size; the px→pt ratio absorbs it.
    let g = geom(0, 0, 1000, 800, 2000, 1600);
    assert_eq!(image_to_screen(&g, 1000, 800), (500, 400));
}

#[test]
fn applies_window_origin_offset() {
    // A window not at the screen origin: image coords are window-relative, the
    // result is absolute screen coords.
    let g = geom(100, 50, 400, 300, 400, 300);
    assert_eq!(image_to_screen(&g, 200, 150), (300, 200));
}

#[test]
fn clamps_out_of_range_pixel_into_window() {
    // A wild guess past the image bounds still lands strictly inside the window.
    let g = geom(100, 50, 400, 300, 400, 300);
    let (x, y) = image_to_screen(&g, 99_999, 99_999);
    assert_eq!((x, y), (499, 349)); // rect_x + w - 1, rect_y + h - 1
}

#[test]
fn clamps_negative_pixel_to_window_origin() {
    let g = geom(100, 50, 400, 300, 400, 300);
    assert_eq!(image_to_screen(&g, -10, -10), (100, 50));
}

#[test]
fn handles_zero_image_dims_without_panicking() {
    // Defensive: a degenerate (0px) image must not divide by zero.
    let g = geom(0, 0, 100, 100, 0, 0);
    let (x, y) = image_to_screen(&g, 10, 10);
    assert!((0..100).contains(&x) && (0..100).contains(&y));
}

// ── parse_locate_response ────────────────────────────────────────────────────

#[test]
fn parses_found_coordinates() {
    let r = parse_locate_response(r#"{"found":true,"x":120,"y":340}"#).unwrap();
    assert_eq!(r, Some((120, 340)));
}

#[test]
fn not_found_is_none() {
    let r = parse_locate_response(r#"{"found":false,"x":0,"y":0}"#).unwrap();
    assert_eq!(r, None);
}

#[test]
fn tolerates_code_fences() {
    let raw = "```json\n{\"found\":true,\"x\":5,\"y\":6}\n```";
    assert_eq!(parse_locate_response(raw).unwrap(), Some((5, 6)));
}

#[test]
fn tolerates_surrounding_prose() {
    let raw = "Sure! Here it is: {\"found\":true,\"x\":7,\"y\":8} — hope that helps";
    assert_eq!(parse_locate_response(raw).unwrap(), Some((7, 8)));
}

#[test]
fn garbage_is_error() {
    assert!(parse_locate_response("no json here").is_err());
}

#[test]
fn missing_found_defaults_to_not_found() {
    // `found` defaults to false when absent, so an answer without it is None.
    let r = parse_locate_response(r#"{"x":1,"y":2}"#).unwrap();
    assert_eq!(r, None);
}

// ── build_locate_user ────────────────────────────────────────────────────────

#[test]
fn user_turn_embeds_description_and_image_marker() {
    let user = build_locate_user("the green Call button", "data:image/png;base64,AAA");
    assert!(user.contains("the green Call button"));
    assert!(user.contains("[IMAGE:data:image/png;base64,AAA]"));
}

// ── image_dims_from_data_uri ─────────────────────────────────────────────────

fn png_data_uri(w: u32, h: u32) -> String {
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
    let img = image::DynamicImage::ImageRgb8(image::RgbImage::new(w, h));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    format!(
        "data:image/png;base64,{}",
        BASE64_STANDARD.encode(buf.get_ref())
    )
}

#[test]
fn reads_png_dimensions() {
    let uri = png_data_uri(640, 480);
    assert_eq!(image_dims_from_data_uri(&uri).unwrap(), (640, 480));
}

#[test]
fn rejects_malformed_data_uri() {
    assert!(image_dims_from_data_uri("not-a-data-uri").is_err());
}
