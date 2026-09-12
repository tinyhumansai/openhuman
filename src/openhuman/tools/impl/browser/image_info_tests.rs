use super::*;
use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

fn test_security() -> Arc<SecurityPolicy> {
    Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::Full,
        workspace_dir: std::env::temp_dir(),
        workspace_only: false,
        forbidden_paths: vec![],
        ..SecurityPolicy::default()
    })
}

#[test]
fn image_info_tool_name() {
    let tool = ImageInfoTool::new(test_security());
    assert_eq!(tool.name(), "image_info");
}

#[test]
fn image_info_tool_description() {
    let tool = ImageInfoTool::new(test_security());
    assert!(!tool.description().is_empty());
    assert!(tool.description().contains("image"));
}

#[test]
fn image_info_tool_schema() {
    let tool = ImageInfoTool::new(test_security());
    let schema = tool.parameters_schema();
    assert!(schema["properties"]["path"].is_object());
    assert!(schema["properties"]["include_base64"].is_object());
    let required = schema["required"].as_array().unwrap();
    assert!(required.contains(&json!("path")));
}

#[test]
fn image_info_tool_spec() {
    let tool = ImageInfoTool::new(test_security());
    let spec = tool.spec();
    assert_eq!(spec.name, "image_info");
    assert!(spec.parameters.is_object());
}

// ── Format detection ────────────────────────────────────────

#[test]
fn detect_png() {
    let bytes = b"\x89PNG\r\n\x1a\n";
    assert_eq!(ImageInfoTool::detect_format(bytes), "png");
}

#[test]
fn detect_jpeg() {
    let bytes = b"\xFF\xD8\xFF\xE0";
    assert_eq!(ImageInfoTool::detect_format(bytes), "jpeg");
}

#[test]
fn detect_gif() {
    let bytes = b"GIF89a";
    assert_eq!(ImageInfoTool::detect_format(bytes), "gif");
}

#[test]
fn detect_webp() {
    let bytes = b"RIFF\x00\x00\x00\x00WEBP";
    assert_eq!(ImageInfoTool::detect_format(bytes), "webp");
}

#[test]
fn detect_bmp() {
    let bytes = b"BM\x00\x00";
    assert_eq!(ImageInfoTool::detect_format(bytes), "bmp");
}

#[test]
fn detect_unknown_short() {
    let bytes = b"\x00\x01";
    assert_eq!(ImageInfoTool::detect_format(bytes), "unknown");
}

#[test]
fn detect_unknown_garbage() {
    let bytes = b"this is not an image";
    assert_eq!(ImageInfoTool::detect_format(bytes), "unknown");
}

// ── Dimension extraction ────────────────────────────────────

#[test]
fn png_dimensions() {
    // Minimal PNG IHDR: 8-byte signature + 4-byte length + 4-byte IHDR + 4-byte width + 4-byte height
    let mut bytes = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // PNG signature
        0x00, 0x00, 0x00, 0x0D, // IHDR length
        0x49, 0x48, 0x44, 0x52, // "IHDR"
        0x00, 0x00, 0x03, 0x20, // width: 800
        0x00, 0x00, 0x02, 0x58, // height: 600
    ];
    bytes.extend_from_slice(&[0u8; 10]); // padding
    let dims = ImageInfoTool::extract_dimensions(&bytes, "png");
    assert_eq!(dims, Some((800, 600)));
}

#[test]
fn gif_dimensions() {
    let bytes = [
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, // GIF89a
        0x40, 0x01, // width: 320 (LE)
        0xF0, 0x00, // height: 240 (LE)
    ];
    let dims = ImageInfoTool::extract_dimensions(&bytes, "gif");
    assert_eq!(dims, Some((320, 240)));
}

#[test]
fn bmp_dimensions() {
    let mut bytes = vec![0u8; 26];
    bytes[0] = b'B';
    bytes[1] = b'M';
    // width at offset 18 (LE): 1024
    bytes[18] = 0x00;
    bytes[19] = 0x04;
    bytes[20] = 0x00;
    bytes[21] = 0x00;
    // height at offset 22 (LE): 768
    bytes[22] = 0x00;
    bytes[23] = 0x03;
    bytes[24] = 0x00;
    bytes[25] = 0x00;
    let dims = ImageInfoTool::extract_dimensions(&bytes, "bmp");
    assert_eq!(dims, Some((1024, 768)));
}

#[test]
fn jpeg_dimensions() {
    // Minimal JPEG-like byte sequence with SOF0 marker
    let mut bytes: Vec<u8> = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, // APP0 marker
        0x00, 0x10, // APP0 length = 16
    ];
    bytes.extend_from_slice(&[0u8; 14]); // APP0 payload
    bytes.extend_from_slice(&[
        0xFF, 0xC0, // SOF0 marker
        0x00, 0x11, // SOF0 length
        0x08, // precision
        0x01, 0xE0, // height: 480
        0x02, 0x80, // width: 640
    ]);
    let dims = ImageInfoTool::extract_dimensions(&bytes, "jpeg");
    assert_eq!(dims, Some((640, 480)));
}

#[test]
fn jpeg_malformed_zero_length_segment() {
    // Zero-length segment should return None instead of looping forever
    let bytes: Vec<u8> = vec![
        0xFF, 0xD8, // SOI
        0xFF, 0xE0, // APP0 marker
        0x00, 0x00, // length = 0 (malformed)
    ];
    let dims = ImageInfoTool::extract_dimensions(&bytes, "jpeg");
    assert!(dims.is_none());
}

#[test]
fn unknown_format_no_dimensions() {
    let bytes = b"random data here";
    let dims = ImageInfoTool::extract_dimensions(bytes, "unknown");
    assert!(dims.is_none());
}

// ── Execute tests ───────────────────────────────────────────

#[tokio::test]
async fn execute_missing_path() {
    let tool = ImageInfoTool::new(test_security());
    let result = tool.execute(json!({})).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn execute_nonexistent_file() {
    let tool = ImageInfoTool::new(test_security());
    let result = tool
        .execute(json!({"path": "nonexistent_image_xyz.png"}))
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.output().contains("not allowed") || result.output().contains("Failed to resolve"),
        "unexpected error: {}",
        result.output()
    );
}

#[tokio::test]
async fn execute_real_file() {
    // Create a minimal valid PNG
    let dir = std::env::temp_dir().join("openhuman_image_info_test");
    let _ = tokio::fs::create_dir_all(&dir).await;
    let png_path = dir.join("test.png");

    // Minimal 1x1 red PNG (67 bytes)
    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, // IHDR length
        0x49, 0x48, 0x44, 0x52, // IHDR
        0x00, 0x00, 0x00, 0x01, // width: 1
        0x00, 0x00, 0x00, 0x01, // height: 1
        0x08, 0x02, 0x00, 0x00, 0x00, // bit depth, color type, etc.
        0x90, 0x77, 0x53, 0xDE, // CRC
        0x00, 0x00, 0x00, 0x0C, // IDAT length
        0x49, 0x44, 0x41, 0x54, // IDAT
        0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC,
        0x33, // CRC
        0x00, 0x00, 0x00, 0x00, // IEND length
        0x49, 0x45, 0x4E, 0x44, // IEND
        0xAE, 0x42, 0x60, 0x82, // CRC
    ];
    tokio::fs::write(&png_path, &png_bytes).await.unwrap();

    let tool = ImageInfoTool::new(test_security());
    let result = tool
        .execute(json!({"path": png_path.to_string_lossy()}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("Format: png"));
    assert!(result.output().contains("Dimensions: 1x1"));
    assert!(!result.output().contains("data:"));

    // Clean up
    let _ = tokio::fs::remove_dir_all(&dir).await;
}

/// A decompression-bomb-shaped PNG (a header claiming an enormous canvas,
/// backed by a file well under `MAX_IMAGE_BYTES`) must not blow up memory or
/// hang: `extract_dimensions` reads four fixed IHDR bytes and never inflates
/// pixel data, so the claimed 60000x60000 canvas is read as fast as any other
/// header and no allocation is proportional to it.
#[tokio::test]
async fn execute_reports_a_bomb_shaped_header_without_decoding_pixels() {
    let dir = std::env::temp_dir().join("openhuman_image_info_bomb");
    let _ = tokio::fs::create_dir_all(&dir).await;
    let png_path = dir.join("bomb.png");

    let mut bytes = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, // signature
        0x00, 0x00, 0x00, 0x0D, // IHDR length
        0x49, 0x48, 0x44, 0x52, // "IHDR"
        0x00, 0x00, 0xEA, 0x60, // width: 60000
        0x00, 0x00, 0xEA, 0x60, // height: 60000
    ];
    bytes.extend_from_slice(&[0u8; 10]); // pad past the 24 bytes IHDR parsing reads
    tokio::fs::write(&png_path, &bytes).await.unwrap();

    let tool = ImageInfoTool::new(test_security());
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tool.execute(json!({"path": png_path.to_string_lossy()})),
    )
    .await
    .expect("image-info execution must settle within two seconds")
    .unwrap();
    assert!(!result.is_error, "{}", result.output());
    assert!(result.output().contains("Dimensions: 60000x60000"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}

#[tokio::test]
async fn execute_with_base64() {
    let dir = std::env::temp_dir().join("openhuman_image_info_b64");
    let _ = tokio::fs::create_dir_all(&dir).await;
    let png_path = dir.join("test_b64.png");

    // Minimal 1x1 PNG
    let png_bytes: Vec<u8> = vec![
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC, 0x33, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    tokio::fs::write(&png_path, &png_bytes).await.unwrap();

    let tool = ImageInfoTool::new(test_security());
    let result = tool
        .execute(json!({"path": png_path.to_string_lossy(), "include_base64": true}))
        .await
        .unwrap();
    assert!(!result.is_error);
    assert!(result.output().contains("data:image/png;base64,"));

    let _ = tokio::fs::remove_dir_all(&dir).await;
}
