use super::traits::{Tool, ToolResult};
use crate::openhuman::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;
use std::path::Path;
use std::sync::Arc;

/// Maximum file size we will read and base64-encode (5 MB).
const MAX_IMAGE_BYTES: u64 = 5_242_880;

/// Tool to read image metadata and optionally return base64-encoded data.
///
/// Since providers are currently text-only, this tool extracts what it can
/// (file size, format, dimensions from header bytes) and provides base64
/// data for future multimodal provider support.
pub struct ImageInfoTool {
    security: Arc<SecurityPolicy>,
}

impl ImageInfoTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// Detect image format from first few bytes (magic numbers).
    fn detect_format(bytes: &[u8]) -> &'static str {
        if bytes.len() < 4 {
            return "unknown";
        }
        if bytes.starts_with(b"\x89PNG") {
            "png"
        } else if bytes.starts_with(b"\xFF\xD8\xFF") {
            "jpeg"
        } else if bytes.starts_with(b"GIF8") {
            "gif"
        } else if bytes.starts_with(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
            "webp"
        } else if bytes.starts_with(b"BM") {
            "bmp"
        } else {
            "unknown"
        }
    }

    /// Try to extract dimensions from image header bytes.
    /// Returns (width, height) if detectable.
    fn extract_dimensions(bytes: &[u8], format: &str) -> Option<(u32, u32)> {
        match format {
            "png" => {
                // PNG IHDR chunk: bytes 16-19 = width, 20-23 = height (big-endian)
                if bytes.len() >= 24 {
                    let w = u32::from_be_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]);
                    let h = u32::from_be_bytes([bytes[20], bytes[21], bytes[22], bytes[23]]);
                    Some((w, h))
                } else {
                    None
                }
            }
            "gif" => {
                // GIF: bytes 6-7 = width, 8-9 = height (little-endian)
                if bytes.len() >= 10 {
                    let w = u32::from(u16::from_le_bytes([bytes[6], bytes[7]]));
                    let h = u32::from(u16::from_le_bytes([bytes[8], bytes[9]]));
                    Some((w, h))
                } else {
                    None
                }
            }
            "bmp" => {
                // BMP: bytes 18-21 = width, 22-25 = height (little-endian, signed)
                if bytes.len() >= 26 {
                    let w = u32::from_le_bytes([bytes[18], bytes[19], bytes[20], bytes[21]]);
                    let h_raw = i32::from_le_bytes([bytes[22], bytes[23], bytes[24], bytes[25]]);
                    let h = h_raw.unsigned_abs();
                    Some((w, h))
                } else {
                    None
                }
            }
            "jpeg" => Self::jpeg_dimensions(bytes),
            _ => None,
        }
    }

    /// Parse JPEG SOF markers to extract dimensions.
    fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
        let mut i = 2; // skip SOI marker
        while i + 1 < bytes.len() {
            if bytes[i] != 0xFF {
                return None;
            }
            let marker = bytes[i + 1];
            i += 2;

            // SOF0..SOF3 markers contain dimensions
            if (0xC0..=0xC3).contains(&marker) {
                if i + 7 <= bytes.len() {
                    let h = u32::from(u16::from_be_bytes([bytes[i + 3], bytes[i + 4]]));
                    let w = u32::from(u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]));
                    return Some((w, h));
                }
                return None;
            }

            // Skip this segment
            if i + 1 < bytes.len() {
                let seg_len = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
                if seg_len < 2 {
                    return None; // Malformed segment (valid segments have length >= 2)
                }
                i += seg_len;
            } else {
                return None;
            }
        }
        None
    }
}

#[async_trait]
impl Tool for ImageInfoTool {
    fn name(&self) -> &str {
        "image_info"
    }

    fn description(&self) -> &str {
        "Read image file metadata (format, dimensions, size) and optionally return base64-encoded data."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the image file (absolute or relative to workspace)"
                },
                "include_base64": {
                    "type": "boolean",
                    "description": "Include base64-encoded image data in output (default: false)"
                }
            },
            "required": ["path"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let path_str = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'path' parameter"))?;

        let include_base64 = args
            .get("include_base64")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);

        let path = Path::new(path_str);

        // Restrict reads to workspace directory to prevent arbitrary file exfiltration
        if !self.security.is_path_allowed(path_str) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Path not allowed: {path_str} (must be within workspace)"
                )),
            });
        }

        if !path.exists() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("File not found: {path_str}")),
            });
        }

        let metadata = tokio::fs::metadata(path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file metadata: {e}"))?;

        let file_size = metadata.len();

        if file_size > MAX_IMAGE_BYTES {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!(
                    "Image too large: {file_size} bytes (max {MAX_IMAGE_BYTES} bytes)"
                )),
            });
        }

        let bytes = tokio::fs::read(path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read image file: {e}"))?;

        let format = Self::detect_format(&bytes);
        let dimensions = Self::extract_dimensions(&bytes, format);

        let mut output = format!("File: {path_str}\nFormat: {format}\nSize: {file_size} bytes");

        if let Some((w, h)) = dimensions {
            let _ = write!(output, "\nDimensions: {w}x{h}");
        }

        if include_base64 {
            use base64::Engine;
            let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
            let mime = match format {
                "png" => "image/png",
                "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                _ => "application/octet-stream",
            };
            let _ = write!(output, "\ndata:{mime};base64,{encoded}");
        }

        Ok(ToolResult {
            success: true,
            output,
            error: None,
        })
    }
}

#[cfg(test)]
mod tests {
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
            .execute(json!({"path": "/tmp/nonexistent_image_xyz.png"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("not found"));
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
            0x08, 0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21,
            0xBC, 0x33, // CRC
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
        assert!(result.success);
        assert!(result.output.contains("Format: png"));
        assert!(result.output.contains("Dimensions: 1x1"));
        assert!(!result.output.contains("data:"));

        // Clean up
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn execute_with_base64() {
        let dir = std::env::temp_dir().join("openhuman_image_info_b64");
        let _ = tokio::fs::create_dir_all(&dir).await;
        let png_path = dir.join("test_b64.png");

        // Minimal 1x1 PNG
        let png_bytes: Vec<u8> = vec![
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0xE2, 0x21, 0xBC,
            0x33, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        tokio::fs::write(&png_path, &png_bytes).await.unwrap();

        let tool = ImageInfoTool::new(test_security());
        let result = tool
            .execute(json!({"path": png_path.to_string_lossy(), "include_base64": true}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("data:image/png;base64,"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
