use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::fmt::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Maximum time to wait for a screenshot command to complete.
const SCREENSHOT_TIMEOUT_SECS: u64 = 15;

/// Tool for capturing screenshots using platform-native commands.
///
/// macOS: `screencapture`
/// Linux: tries `gnome-screenshot`, `scrot`, `import` (`ImageMagick`) in order.
pub struct ScreenshotTool {
    security: Arc<SecurityPolicy>,
}

impl ScreenshotTool {
    pub fn new(security: Arc<SecurityPolicy>) -> Self {
        Self { security }
    }

    /// Determine the screenshot command for the current platform.
    fn screenshot_command(output_path: &str) -> Option<Vec<String>> {
        if std::env::consts::OS == "macos" {
            Some(vec![
                "screencapture".into(),
                "-x".into(), // no sound
                output_path.into(),
            ])
        } else if std::env::consts::OS == "linux" {
            Some(vec![
                "sh".into(),
                "-c".into(),
                format!(
                    "if command -v gnome-screenshot >/dev/null 2>&1; then \
                         gnome-screenshot -f '{output_path}'; \
                     elif command -v scrot >/dev/null 2>&1; then \
                         scrot '{output_path}'; \
                     elif command -v import >/dev/null 2>&1; then \
                         import -window root '{output_path}'; \
                     else \
                         echo 'NO_SCREENSHOT_TOOL' >&2; exit 1; \
                     fi"
                ),
            ])
        } else {
            None
        }
    }

    /// Execute the screenshot capture and return the result.
    async fn capture(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
        let filename = args
            .get("filename")
            .and_then(|v| v.as_str())
            .map_or_else(|| format!("screenshot_{timestamp}.png"), String::from);

        // Sanitize filename to prevent path traversal
        let safe_name = PathBuf::from(&filename).file_name().map_or_else(
            || format!("screenshot_{timestamp}.png"),
            |n| n.to_string_lossy().to_string(),
        );

        // Reject filenames with shell-breaking characters to prevent injection in sh -c
        const SHELL_UNSAFE: &[char] = &[
            '\'', '"', '`', '$', '\\', ';', '|', '&', '\n', '\0', '(', ')',
        ];
        if safe_name.contains(SHELL_UNSAFE) {
            return Ok(ToolResult::error(
                "Filename contains characters unsafe for shell execution",
            ));
        }

        let output_path = self.security.workspace_dir.join(&safe_name);
        let output_str = output_path.to_string_lossy().to_string();

        let Some(mut cmd_args) = Self::screenshot_command(&output_str) else {
            return Ok(ToolResult::error(
                "Screenshot not supported on this platform",
            ));
        };

        // macOS region flags
        if std::env::consts::OS == "macos" {
            if let Some(region) = args.get("region").and_then(|v| v.as_str()) {
                match region {
                    "selection" => cmd_args.insert(1, "-s".into()),
                    "window" => cmd_args.insert(1, "-w".into()),
                    _ => {} // ignore unknown regions
                }
            }
        }

        let program = cmd_args.remove(0);
        let result = tokio::time::timeout(
            Duration::from_secs(SCREENSHOT_TIMEOUT_SECS),
            tokio::process::Command::new(&program)
                .args(&cmd_args)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if stderr.contains("NO_SCREENSHOT_TOOL") {
                        return Ok(ToolResult::error(
                                "No screenshot tool found. Install gnome-screenshot, scrot, or ImageMagick.",
                            ));
                    }
                    return Ok(ToolResult::error(format!(
                        "Screenshot command failed: {stderr}"
                    )));
                }

                Self::read_and_encode(&output_path).await
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!(
                "Failed to execute screenshot command: {e}"
            ))),
            Err(_) => Ok(ToolResult::error(format!(
                "Screenshot timed out after {SCREENSHOT_TIMEOUT_SECS}s"
            ))),
        }
    }

    /// Read the screenshot file and return a base64 data-URL the model can see.
    ///
    /// Full-screen Retina captures are multi-MB PNGs that blow the inline
    /// budget. Rather than dropping the image (which leaves vision-driven
    /// control blind), downscale oversized captures to a JPEG that fits — the
    /// model can then actually see the screen. Reports the *shown* dimensions so
    /// callers know the coordinate space they're reading.
    async fn read_and_encode(output_path: &std::path::Path) -> anyhow::Result<ToolResult> {
        // ~1.5 MB raw → ~2 MB base64, a safe inline payload size.
        const MAX_RAW_BYTES: usize = 1_572_864;

        let bytes = match tokio::fs::read(output_path).await {
            Ok(b) => b,
            Err(e) => {
                return Ok(ToolResult::error(format!(
                    "Failed to read screenshot file: {e}"
                )))
            }
        };
        let ext = output_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_lowercase();

        // Fits as-is → return verbatim.
        if bytes.len() <= MAX_RAW_BYTES {
            let mime = match ext.as_str() {
                "jpg" | "jpeg" => "image/jpeg",
                "bmp" => "image/bmp",
                "gif" => "image/gif",
                "webp" => "image/webp",
                _ => "image/png",
            };
            return Ok(Self::data_url_result(output_path, &bytes, mime, None));
        }

        // Too large → downscale to a JPEG that fits (CPU work off the runtime).
        match tokio::task::spawn_blocking(move || downscale_to_jpeg(&bytes, MAX_RAW_BYTES)).await {
            Ok(Ok((jpeg, w, h))) => Ok(Self::data_url_result(
                output_path,
                &jpeg,
                "image/jpeg",
                Some((w, h)),
            )),
            Ok(Err(e)) => Ok(ToolResult::success(format!(
                "Screenshot saved to: {} (could not downscale for inline view: {e})",
                output_path.display()
            ))),
            Err(e) => Ok(ToolResult::error(format!("downscale task failed: {e}"))),
        }
    }

    /// Build a success result carrying a base64 data-URL of `data`.
    fn data_url_result(
        output_path: &std::path::Path,
        data: &[u8],
        mime: &str,
        shown_dims: Option<(u32, u32)>,
    ) -> ToolResult {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        let mut msg = format!("Screenshot saved to: {}\n", output_path.display());
        if let Some((w, h)) = shown_dims {
            let _ = write!(
                msg,
                "Downscaled to {w}x{h}px for inline view (coordinates you read are in this {w}x{h} space).\n"
            );
        }
        let _ = write!(msg, "data:{mime};base64,{encoded}");
        ToolResult::success(msg)
    }
}

/// Decode image bytes, downscale (preserving aspect ratio), and JPEG-encode so
/// the result is ≤ `max_bytes`. Returns `(jpeg_bytes, width, height)`.
fn downscale_to_jpeg(bytes: &[u8], max_bytes: usize) -> Result<(Vec<u8>, u32, u32), String> {
    let img = image::load_from_memory(bytes).map_err(|e| format!("decode: {e}"))?;
    let mut last: Option<(Vec<u8>, u32, u32)> = None;
    for max_dim in [1568u32, 1280, 1024, 768, 600] {
        // Drop alpha before JPEG-encoding: JPEG has no alpha channel, so an
        // RGBA capture (PNG screenshots often carry one) would otherwise fail
        // to encode and leave vision-driven control blind.
        let thumb = img.thumbnail(max_dim, max_dim).to_rgb8(); // fits within max_dim², keeps aspect
        let (w, h) = (thumb.width(), thumb.height());
        let mut buf = std::io::Cursor::new(Vec::new());
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, 72)
            .encode_image(&thumb)
            .map_err(|e| format!("jpeg encode: {e}"))?;
        let out = buf.into_inner();
        if out.len() <= max_bytes {
            return Ok((out, w, h));
        }
        last = Some((out, w, h));
    }
    last.ok_or_else(|| "could not produce a fitting JPEG".to_string())
}

#[async_trait]
impl Tool for ScreenshotTool {
    fn name(&self) -> &str {
        "screenshot"
    }

    fn description(&self) -> &str {
        "Capture a screenshot of the current screen. Returns the file path and base64-encoded PNG data."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "Optional filename (default: screenshot_<timestamp>.png). Saved in workspace."
                },
                "region": {
                    "type": "string",
                    "description": "Optional region for macOS: 'selection' for interactive crop, 'window' for front window. Ignored on Linux."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        if !self.security.can_act() {
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }
        self.capture(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    #[test]
    fn downscale_to_jpeg_shrinks_oversized_capture() {
        // A 1600x1200 PNG of noise is well over a tight budget; downscaling must
        // produce a smaller JPEG that still decodes, so the model can see it.
        let mut img = image::RgbImage::new(1600, 1200);
        for (i, px) in img.pixels_mut().enumerate() {
            *px = image::Rgb([(i % 251) as u8, (i % 253) as u8, (i % 247) as u8]);
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode png");
        let png = png.into_inner();

        let max = 400_000usize;
        let (jpeg, w, h) = downscale_to_jpeg(&png, max).expect("downscale");
        assert!(jpeg.len() <= max, "jpeg {} should be <= {max}", jpeg.len());
        assert!(
            w <= 1568 && h <= 1568,
            "dims {w}x{h} should be capped to 1568"
        );
        assert!(
            jpeg.len() < png.len(),
            "jpeg should be smaller than source png"
        );
        // Result must be a valid, decodable image at the reported dims.
        let decoded = image::load_from_memory(&jpeg).expect("jpeg decodes");
        assert_eq!((decoded.width(), decoded.height()), (w, h));
    }

    #[test]
    fn downscale_to_jpeg_handles_rgba_input() {
        // PNG screenshots frequently carry an alpha channel. JPEG has none, so
        // the encoder must run on RGB — otherwise an RGBA capture fails to
        // encode and leaves vision-driven control blind.
        let mut img = image::RgbaImage::new(1600, 1200);
        for (i, px) in img.pixels_mut().enumerate() {
            *px = image::Rgba([(i % 251) as u8, (i % 253) as u8, (i % 247) as u8, 128]);
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .expect("encode png");
        let png = png.into_inner();

        let (jpeg, w, h) = downscale_to_jpeg(&png, 400_000).expect("rgba downscales");
        let decoded = image::load_from_memory(&jpeg).expect("jpeg decodes");
        assert_eq!((decoded.width(), decoded.height()), (w, h));
    }

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Full,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn screenshot_tool_name() {
        let tool = ScreenshotTool::new(test_security());
        assert_eq!(tool.name(), "screenshot");
    }

    #[test]
    fn screenshot_tool_description() {
        let tool = ScreenshotTool::new(test_security());
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("screenshot"));
    }

    #[test]
    fn screenshot_tool_schema() {
        let tool = ScreenshotTool::new(test_security());
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["filename"].is_object());
        assert!(schema["properties"]["region"].is_object());
    }

    #[test]
    fn screenshot_tool_spec() {
        let tool = ScreenshotTool::new(test_security());
        let spec = tool.spec();
        assert_eq!(spec.name, "screenshot");
        assert!(spec.parameters.is_object());
    }

    #[test]
    fn screenshot_command_exists() {
        if !matches!(std::env::consts::OS, "macos" | "linux") {
            return;
        }
        let cmd = ScreenshotTool::screenshot_command("/tmp/test.png");
        assert!(cmd.is_some());
        let args = cmd.unwrap();
        assert!(!args.is_empty());
    }

    #[tokio::test]
    async fn screenshot_rejects_shell_injection_filename() {
        let tool = ScreenshotTool::new(test_security());
        let result = tool
            .execute(json!({"filename": "test'injection.png"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("unsafe for shell execution"));
    }

    #[test]
    fn screenshot_command_contains_output_path() {
        if !matches!(std::env::consts::OS, "macos" | "linux") {
            return;
        }
        let cmd = ScreenshotTool::screenshot_command("/tmp/my_screenshot.png").unwrap();
        let joined = cmd.join(" ");
        assert!(
            joined.contains("/tmp/my_screenshot.png"),
            "Command should contain the output path"
        );
    }

    // ── execute blocked in read-only autonomy ─────────────────────────────────

    #[tokio::test]
    async fn screenshot_blocked_in_read_only_mode() {
        use crate::openhuman::security::AutonomyLevel;
        let readonly = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        });
        let tool = ScreenshotTool::new(readonly);
        let result = tool.execute(serde_json::json!({})).await.unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("read-only"));
    }

    // ── screenshot_command on unsupported platform returns None ───────────────

    #[test]
    fn screenshot_command_returns_none_for_unsupported_os() {
        let result = ScreenshotTool::screenshot_command("/tmp/test.png");
        if cfg!(any(target_os = "macos", target_os = "linux")) {
            assert!(
                result.is_some(),
                "macOS/Linux must produce a screenshot command"
            );
        } else {
            assert_eq!(
                result, None,
                "unsupported platforms must return None (no panic)"
            );
        }
    }

    // ── safe filename that has no shell-unsafe chars is allowed ──────────────

    #[tokio::test]
    async fn screenshot_accepts_safe_filename() {
        // On unsupported platforms the tool will return an error about platform
        // support, not about the filename being unsafe.  We just check there is
        // no "unsafe for shell execution" error.
        let tool = ScreenshotTool::new(test_security());
        let result = tool
            .execute(serde_json::json!({"filename": "safe_name.png"}))
            .await
            .unwrap();
        if result.is_error {
            assert!(
                !result.output().contains("unsafe for shell execution"),
                "safe filename should not trigger shell-injection guard, got: {}",
                result.output()
            );
        }
    }

    // ── multiple unsafe chars are all rejected ────────────────────────────────

    #[tokio::test]
    async fn screenshot_rejects_all_unsafe_chars() {
        let tool = ScreenshotTool::new(test_security());
        // Backslash is a path separator on Windows, not a shell-injection risk there.
        let mut chars = vec!['\'', '"', '`', '$', ';', '|', '&', '(', ')'];
        if matches!(std::env::consts::OS, "macos" | "linux") {
            chars.push('\\');
        }
        for ch in chars {
            let filename = format!("test{ch}name.png");
            let result = tool
                .execute(serde_json::json!({"filename": filename}))
                .await
                .unwrap();
            assert!(
                result.is_error,
                "expected error for filename with char '{ch}', got success"
            );
            assert!(
                result.output().contains("unsafe for shell execution"),
                "unexpected error message for char '{ch}': {}",
                result.output()
            );
        }
    }

    // ── read_and_encode: file not found returns error ─────────────────────────

    #[tokio::test]
    async fn read_and_encode_file_not_found_returns_error() {
        let result = ScreenshotTool::read_and_encode(std::path::Path::new(
            "/tmp/openhuman_test_nonexistent_12345.png",
        ))
        .await
        .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("Failed to read screenshot file"));
    }

    // ── read_and_encode: file within size limit is base64-encoded ─────────────

    #[tokio::test]
    async fn read_and_encode_small_file_is_encoded() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.png");
        let mut f = tokio::fs::File::create(&path).await.unwrap();
        // Minimal valid bytes (not a real PNG but enough for the encoding test)
        f.write_all(b"\x89PNG\r\n\x1a\n").await.unwrap();
        drop(f);

        let result = ScreenshotTool::read_and_encode(&path).await.unwrap();
        assert!(!result.is_error);
        assert!(
            result.output().contains("data:image/png;base64,"),
            "output should contain base64 data URL"
        );
        assert!(
            result.output().contains("Screenshot saved to:"),
            "output should contain saved path"
        );
    }

    // ── read_and_encode: JPEG extension picks correct MIME type ───────────────

    #[tokio::test]
    async fn read_and_encode_jpeg_uses_jpeg_mime() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("image.jpg");
        let mut f = tokio::fs::File::create(&path).await.unwrap();
        f.write_all(b"\xFF\xD8\xFF").await.unwrap();
        drop(f);

        let result = ScreenshotTool::read_and_encode(&path).await.unwrap();
        assert!(!result.is_error);
        assert!(result.output().contains("data:image/jpeg;base64,"));
    }

    // ── read_and_encode: large file returns saved-path-only message ───────────

    #[tokio::test]
    async fn read_and_encode_large_file_downscales_to_viewable_jpeg() {
        // A large *real* PNG (over MAX_RAW_BYTES) must be downscaled to an inline
        // JPEG data-URL the model can see — not dropped (the old behavior left
        // vision-driven control blind).
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.png");
        let mut img = image::RgbImage::new(2200, 1500);
        for (i, px) in img.pixels_mut().enumerate() {
            *px = image::Rgb([(i % 251) as u8, (i % 253) as u8, (i % 247) as u8]);
        }
        image::DynamicImage::ImageRgb8(img)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        assert!(
            tokio::fs::metadata(&path).await.unwrap().len() > 1_572_864,
            "test PNG should exceed the inline budget"
        );

        let result = ScreenshotTool::read_and_encode(&path).await.unwrap();
        assert!(
            !result.is_error,
            "should not error, got: {}",
            result.output()
        );
        let out = result.output();
        assert!(
            out.contains("data:image/jpeg;base64,"),
            "should inline a jpeg: {out}"
        );
        assert!(
            out.contains("Downscaled to"),
            "should report downscale: {out}"
        );
    }
}
