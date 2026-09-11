use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;
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

        // Security check: validate path string, resolve symlinks, confirm workspace containment.
        let resolved = match self.security.validate_path(path_str).await {
            Ok(p) => p,
            Err(msg) => return Ok(ToolResult::error(msg)),
        };

        let metadata = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file metadata: {e}"))?;

        let file_size = metadata.len();

        if file_size > MAX_IMAGE_BYTES {
            return Ok(ToolResult::error(format!(
                "Image too large: {file_size} bytes (max {MAX_IMAGE_BYTES} bytes)"
            )));
        }

        let bytes = tokio::fs::read(&resolved)
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

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
#[path = "image_info_tests.rs"]
mod tests;
