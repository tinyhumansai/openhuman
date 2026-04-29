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
/// Maximum base64 payload size to return (2 MB of base64 ≈ 1.5 MB image).
const MAX_BASE64_BYTES: usize = 2_097_152;

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

    /// Read the screenshot file and return base64-encoded result.
    async fn read_and_encode(output_path: &std::path::Path) -> anyhow::Result<ToolResult> {
        // Check file size before reading to prevent OOM on large screenshots
        const MAX_RAW_BYTES: u64 = 1_572_864; // ~1.5 MB (base64 expands ~33%)
        if let Ok(meta) = tokio::fs::metadata(output_path).await {
            if meta.len() > MAX_RAW_BYTES {
                return Ok(ToolResult::success(format!(
                    "Screenshot saved to: {}\nSize: {} bytes (too large to base64-encode inline)",
                    output_path.display(),
                    meta.len(),
                )));
            }
        }

        match tokio::fs::read(output_path).await {
            Ok(bytes) => {
                use base64::Engine;
                let size = bytes.len();
                let mut encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
                let truncated = if encoded.len() > MAX_BASE64_BYTES {
                    encoded.truncate(encoded.floor_char_boundary(MAX_BASE64_BYTES));
                    true
                } else {
                    false
                };

                let mut output_msg = format!(
                    "Screenshot saved to: {}\nSize: {size} bytes\nBase64 length: {}",
                    output_path.display(),
                    encoded.len(),
                );
                if truncated {
                    output_msg.push_str(" (truncated)");
                }
                let mime = match output_path.extension().and_then(|e| e.to_str()) {
                    Some("jpg" | "jpeg") => "image/jpeg",
                    Some("bmp") => "image/bmp",
                    Some("gif") => "image/gif",
                    Some("webp") => "image/webp",
                    _ => "image/png",
                };
                let _ = write!(output_msg, "\ndata:{mime};base64,{encoded}");

                Ok(ToolResult::success(output_msg))
            }
            Err(e) => Ok(ToolResult::error(format!(
                "Failed to read screenshot file: {e}"
            ))),
        }
    }
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
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }
        self.capture(args).await
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
        if !matches!(std::env::consts::OS, "macos" | "linux") {
            return;
        }
        let tool = ScreenshotTool::new(test_security());
        for ch in ['\'', '"', '`', '$', '\\', ';', '|', '&', '(', ')'] {
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
    async fn read_and_encode_large_file_skips_base64() {
        use tokio::io::AsyncWriteExt;
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("big.png");
        let mut f = tokio::fs::File::create(&path).await.unwrap();
        // Write ~1.6 MB to exceed the MAX_RAW_BYTES threshold (1.5 MB)
        let chunk = vec![0u8; 1024];
        for _ in 0..1600 {
            f.write_all(&chunk).await.unwrap();
        }
        drop(f);

        let result = ScreenshotTool::read_and_encode(&path).await.unwrap();
        assert!(!result.is_error, "large file should not be an error result");
        assert!(
            result.output().contains("too large to base64-encode"),
            "large file should skip base64, got: {}",
            result.output()
        );
    }
}
