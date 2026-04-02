//! Image compression and resizing for screenshot intelligence.
//!
//! Before sending screenshots to the vision LLM we shrink them:
//!   1. Decode the base64 PNG data-URI into pixels.
//!   2. Resize so the longest edge fits within `max_dimension` (default 1024 px).
//!   3. Re-encode as JPEG at a configurable quality (default 72).
//!   4. Return a `data:image/jpeg;base64,…` URI ready for the Ollama vision API.
//!
//! Smaller images mean fewer tokens, faster inference, and lower memory pressure.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageReader};
use std::io::Cursor;

/// Default longest-edge cap (pixels). Vision models rarely benefit from
/// more than 1024 px on the long side for UI-screenshot analysis.
pub(crate) const DEFAULT_MAX_DIMENSION: u32 = 1024;

/// Default JPEG quality (1-100). 72 is a good trade-off: visually acceptable
/// for UI text while cutting size by ~70-85 % compared to PNG.
pub(crate) const DEFAULT_JPEG_QUALITY: u8 = 72;

/// Result of compressing a screenshot.
#[derive(Debug, Clone)]
pub(crate) struct CompressedImage {
    /// `data:image/jpeg;base64,…` ready for the vision API.
    pub data_uri: String,
    /// Original decoded size in bytes (raw PNG payload).
    pub original_bytes: usize,
    /// Compressed JPEG size in bytes.
    pub compressed_bytes: usize,
    /// Original dimensions (width, height).
    pub original_dimensions: (u32, u32),
    /// Final dimensions after resize (width, height).
    pub final_dimensions: (u32, u32),
}

/// Compress and resize a base64 PNG data-URI for vision LLM consumption.
///
/// Accepts a full `data:image/png;base64,…` URI **or** a raw base64 string.
/// Returns `Err` if the payload cannot be decoded as a valid image.
pub(crate) fn compress_screenshot(
    image_ref: &str,
    max_dimension: Option<u32>,
    jpeg_quality: Option<u8>,
) -> Result<CompressedImage, String> {
    let max_dim = max_dimension.unwrap_or(DEFAULT_MAX_DIMENSION).max(64);
    let quality = jpeg_quality.unwrap_or(DEFAULT_JPEG_QUALITY).clamp(10, 100);

    // ── 1. Strip data-URI prefix and decode base64 ──────────────────────
    let b64_payload = strip_data_uri_prefix(image_ref);
    let raw_bytes = B64
        .decode(b64_payload)
        .map_err(|e| format!("base64 decode failed: {e}"))?;
    let original_bytes = raw_bytes.len();

    // ── 2. Decode into pixels ───────────────────────────────────────────
    let img = ImageReader::new(Cursor::new(&raw_bytes))
        .with_guessed_format()
        .map_err(|e| format!("image format detection failed: {e}"))?
        .decode()
        .map_err(|e| format!("image decode failed: {e}"))?;

    let original_dimensions = (img.width(), img.height());

    // ── 3. Resize if needed ─────────────────────────────────────────────
    let resized = resize_to_fit(img, max_dim);
    let final_dimensions = (resized.width(), resized.height());

    // ── 4. Encode as JPEG ───────────────────────────────────────────────
    let jpeg_bytes = encode_jpeg(&resized, quality)?;
    let compressed_bytes = jpeg_bytes.len();

    // ── 5. Build data-URI ───────────────────────────────────────────────
    let b64_out = B64.encode(&jpeg_bytes);
    let data_uri = format!("data:image/jpeg;base64,{b64_out}");

    tracing::debug!(
        "[screen_intelligence] image compressed: {}x{} -> {}x{}, {} -> {} bytes ({:.0}% reduction)",
        original_dimensions.0,
        original_dimensions.1,
        final_dimensions.0,
        final_dimensions.1,
        original_bytes,
        compressed_bytes,
        (1.0 - compressed_bytes as f64 / original_bytes as f64) * 100.0,
    );

    Ok(CompressedImage {
        data_uri,
        original_bytes,
        compressed_bytes,
        original_dimensions,
        final_dimensions,
    })
}

/// Strip common data-URI prefixes, returning the raw base64 payload.
fn strip_data_uri_prefix(input: &str) -> &str {
    // Handle: data:image/png;base64,… | data:image/jpeg;base64,… | data:image/*;base64,…
    if let Some(pos) = input.find(";base64,") {
        &input[pos + 8..]
    } else {
        input
    }
}

/// Resize `img` so neither dimension exceeds `max_dim`, preserving aspect ratio.
/// If both dimensions are already within bounds the image is returned unchanged.
fn resize_to_fit(img: DynamicImage, max_dim: u32) -> DynamicImage {
    let (w, h) = (img.width(), img.height());
    if w <= max_dim && h <= max_dim {
        return img;
    }

    let scale = max_dim as f64 / w.max(h) as f64;
    let new_w = ((w as f64 * scale).round() as u32).max(1);
    let new_h = ((h as f64 * scale).round() as u32).max(1);

    // Lanczos3 gives crisp text edges — important for UI screenshots.
    img.resize_exact(new_w, new_h, FilterType::Lanczos3)
}

/// Encode a `DynamicImage` as JPEG bytes at the given quality.
fn encode_jpeg(img: &DynamicImage, quality: u8) -> Result<Vec<u8>, String> {
    let rgb = img.to_rgb8();
    let mut buf: Vec<u8> = Vec::new();
    let encoder = JpegEncoder::new_with_quality(&mut buf, quality);
    rgb.write_with_encoder(encoder)
        .map_err(|e| format!("JPEG encode failed: {e}"))?;
    Ok(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb, RgbImage};

    /// Helper: create a solid-color PNG image of given dimensions and return
    /// its `data:image/png;base64,…` URI.
    fn make_test_png(width: u32, height: u32, color: [u8; 3]) -> String {
        let img: RgbImage = ImageBuffer::from_fn(width, height, |_, _| Rgb(color));
        let mut png_bytes: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        img.write_with_encoder(encoder).expect("PNG encode");
        let b64 = B64.encode(&png_bytes);
        format!("data:image/png;base64,{b64}")
    }

    // ── Basic compression ───────────────────────────────────────────────

    #[test]
    fn compress_reduces_size_for_large_image() {
        let uri = make_test_png(2048, 1536, [100, 150, 200]);
        let result = compress_screenshot(&uri, None, None).unwrap();

        assert!(
            result.compressed_bytes < result.original_bytes,
            "JPEG should be smaller than PNG for a large solid image"
        );
        assert_eq!(result.original_dimensions, (2048, 1536));
        assert!(
            result.final_dimensions.0 <= DEFAULT_MAX_DIMENSION,
            "width should be capped"
        );
        assert!(
            result.final_dimensions.1 <= DEFAULT_MAX_DIMENSION,
            "height should be capped"
        );
        assert!(result.data_uri.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn compress_preserves_aspect_ratio() {
        let uri = make_test_png(2000, 1000, [255, 0, 0]);
        let result = compress_screenshot(&uri, Some(500), None).unwrap();

        // 2000x1000 → 500x250  (long edge capped at 500)
        assert_eq!(result.final_dimensions.0, 500);
        assert_eq!(result.final_dimensions.1, 250);
    }

    #[test]
    fn compress_portrait_image() {
        let uri = make_test_png(600, 1800, [0, 255, 0]);
        let result = compress_screenshot(&uri, Some(900), None).unwrap();

        // 600x1800 → 300x900  (height is long edge)
        assert_eq!(result.final_dimensions.1, 900);
        assert_eq!(result.final_dimensions.0, 300);
    }

    // ── No-resize path ──────────────────────────────────────────────────

    #[test]
    fn small_image_not_resized() {
        let uri = make_test_png(200, 150, [50, 50, 50]);
        let result = compress_screenshot(&uri, Some(1024), None).unwrap();

        assert_eq!(result.original_dimensions, result.final_dimensions);
    }

    #[test]
    fn exact_max_dimension_not_resized() {
        let uri = make_test_png(1024, 768, [80, 80, 80]);
        let result = compress_screenshot(&uri, Some(1024), None).unwrap();

        assert_eq!(result.final_dimensions, (1024, 768));
    }

    // ── Quality settings ────────────────────────────────────────────────

    #[test]
    fn lower_quality_produces_smaller_output() {
        let uri = make_test_png(800, 600, [128, 64, 200]);
        let high = compress_screenshot(&uri, None, Some(95)).unwrap();
        let low = compress_screenshot(&uri, None, Some(30)).unwrap();

        assert!(
            low.compressed_bytes < high.compressed_bytes,
            "quality 30 should produce smaller JPEG than quality 95"
        );
    }

    #[test]
    fn quality_clamped_to_valid_range() {
        let uri = make_test_png(100, 100, [0, 0, 0]);
        // quality below 10 should be clamped to 10
        let result = compress_screenshot(&uri, None, Some(1)).unwrap();
        assert!(result.compressed_bytes > 0);

        // quality above 100 should be clamped to 100
        let result2 = compress_screenshot(&uri, None, Some(255)).unwrap();
        assert!(result2.compressed_bytes > 0);
    }

    // ── Data-URI prefix handling ────────────────────────────────────────

    #[test]
    fn handles_raw_base64_without_prefix() {
        // Build raw base64 without data URI prefix
        let img: RgbImage = ImageBuffer::from_fn(64, 64, |_, _| Rgb([255, 255, 255]));
        let mut png_bytes: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        img.write_with_encoder(encoder).expect("PNG encode");
        let raw_b64 = B64.encode(&png_bytes);

        let result = compress_screenshot(&raw_b64, None, None).unwrap();
        assert_eq!(result.original_dimensions, (64, 64));
    }

    #[test]
    fn handles_jpeg_data_uri_prefix() {
        // Even if input is labeled as JPEG, we decode by content not prefix
        let img: RgbImage = ImageBuffer::from_fn(64, 64, |_, _| Rgb([100, 100, 100]));
        let mut png_bytes: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        img.write_with_encoder(encoder).expect("PNG encode");
        let b64 = B64.encode(&png_bytes);
        let uri = format!("data:image/jpeg;base64,{b64}");

        let result = compress_screenshot(&uri, None, None).unwrap();
        assert_eq!(result.original_dimensions, (64, 64));
    }

    // ── Edge cases ──────────────────────────────────────────────────────

    #[test]
    fn tiny_1x1_image() {
        let uri = make_test_png(1, 1, [255, 0, 0]);
        let result = compress_screenshot(&uri, None, None).unwrap();

        assert_eq!(result.original_dimensions, (1, 1));
        assert_eq!(result.final_dimensions, (1, 1));
    }

    #[test]
    fn very_wide_panoramic_image() {
        let uri = make_test_png(4000, 100, [0, 0, 255]);
        let result = compress_screenshot(&uri, Some(1024), None).unwrap();

        assert_eq!(result.final_dimensions.0, 1024);
        // 4000x100 → 1024x26 (proportional)
        assert!(result.final_dimensions.1 > 0);
        assert!(result.final_dimensions.1 <= 100);
    }

    #[test]
    fn square_image() {
        let uri = make_test_png(2000, 2000, [128, 128, 128]);
        let result = compress_screenshot(&uri, Some(512), None).unwrap();

        assert_eq!(result.final_dimensions, (512, 512));
    }

    #[test]
    fn invalid_base64_returns_error() {
        let result = compress_screenshot("data:image/png;base64,!!!invalid!!!", None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("base64 decode failed"));
    }

    #[test]
    fn valid_base64_but_not_image_returns_error() {
        let b64 = B64.encode(b"this is not an image");
        let uri = format!("data:image/png;base64,{b64}");
        let result = compress_screenshot(&uri, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn min_max_dimension_floor() {
        // max_dimension below 64 should be clamped to 64
        let uri = make_test_png(200, 200, [0, 0, 0]);
        let result = compress_screenshot(&uri, Some(10), None).unwrap();
        assert!(
            result.final_dimensions.0 >= 64,
            "max_dimension should be floored to 64"
        );
    }

    // ── strip_data_uri_prefix ───────────────────────────────────────────

    #[test]
    fn strip_prefix_png() {
        let input = "data:image/png;base64,ABCD1234";
        assert_eq!(strip_data_uri_prefix(input), "ABCD1234");
    }

    #[test]
    fn strip_prefix_jpeg() {
        let input = "data:image/jpeg;base64,XYZ";
        assert_eq!(strip_data_uri_prefix(input), "XYZ");
    }

    #[test]
    fn strip_prefix_no_prefix() {
        let input = "ABCD1234";
        assert_eq!(strip_data_uri_prefix(input), "ABCD1234");
    }

    // ── Multicolored image (more realistic compression ratio) ───────────

    #[test]
    fn multicolored_image_compresses_well() {
        // Create a gradient image that's more representative of real screenshots
        let width = 1920u32;
        let height = 1080u32;
        let img: RgbImage = ImageBuffer::from_fn(width, height, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
        });
        let mut png_bytes: Vec<u8> = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(&mut png_bytes);
        img.write_with_encoder(encoder).expect("PNG encode");
        let b64 = B64.encode(&png_bytes);
        let uri = format!("data:image/png;base64,{b64}");

        let result = compress_screenshot(&uri, Some(1024), Some(72)).unwrap();

        assert!(result.final_dimensions.0 <= 1024);
        assert!(result.final_dimensions.1 <= 1024);
        // For a gradient image, combined resize+JPEG should give significant savings
        assert!(
            result.compressed_bytes < result.original_bytes / 2,
            "should achieve at least 50% size reduction on a 1920x1080 gradient"
        );
    }
}
