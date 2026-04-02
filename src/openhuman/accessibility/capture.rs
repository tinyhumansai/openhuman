//! Timestamp helper and screen capture via platform-native tools.

use super::types::AppContext;
#[cfg(target_os = "macos")]
use super::{detect_permissions, PermissionState};
#[cfg(target_os = "macos")]
use std::path::{Path, PathBuf};

/// Maximum screenshot size in bytes before downscaling is attempted.
pub const MAX_SCREENSHOT_BYTES: usize = 1_500_000;
#[cfg(target_os = "macos")]
const SCREENSHOT_DOWNSCALE_WIDTH: &str = "1280";

/// Capture mode used for a screenshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureMode {
    Windowed,
    Fullscreen,
}

impl std::fmt::Display for CaptureMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureMode::Windowed => write!(f, "windowed"),
            CaptureMode::Fullscreen => write!(f, "fullscreen"),
        }
    }
}

fn capture_mode_for_context(context: Option<&AppContext>) -> CaptureMode {
    match context.and_then(|ctx| ctx.bounds) {
        Some(bounds) if bounds.width > 0 && bounds.height > 0 => CaptureMode::Windowed,
        _ => CaptureMode::Fullscreen,
    }
}

#[cfg(target_os = "macos")]
fn log_capture_mode_decision(context: Option<&AppContext>, capture_mode: &CaptureMode) {
    match (capture_mode, context.and_then(|ctx| ctx.bounds)) {
        (CaptureMode::Windowed, Some(bounds)) => {
            tracing::debug!(
                "[accessibility] capture mode=windowed rect={},{},{},{} app={:?}",
                bounds.x,
                bounds.y,
                bounds.width,
                bounds.height,
                context.and_then(|ctx| ctx.app_name.as_deref())
            );
        }
        (CaptureMode::Windowed, None) => {
            tracing::debug!(
                "[accessibility] capture mode resolved to windowed without bounds; treating as fullscreen fallback"
            );
        }
        (CaptureMode::Fullscreen, Some(bounds)) => {
            tracing::debug!(
                "[accessibility] invalid bounds ({}x{}), falling back to fullscreen",
                bounds.width,
                bounds.height
            );
        }
        (CaptureMode::Fullscreen, None) => {
            tracing::debug!(
                "[accessibility] no window bounds available, falling back to fullscreen"
            );
        }
    }
}

#[cfg(target_os = "macos")]
fn downscale_width_for_capture(
    bytes_len: usize,
    _capture_mode: &CaptureMode,
) -> Option<&'static str> {
    (bytes_len > MAX_SCREENSHOT_BYTES).then_some(SCREENSHOT_DOWNSCALE_WIDTH)
}

#[cfg(target_os = "macos")]
struct TemporaryScreenshotFile {
    path: PathBuf,
}

#[cfg(target_os = "macos")]
impl TemporaryScreenshotFile {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(target_os = "macos")]
impl Drop for TemporaryScreenshotFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn capture_screen_image_ref_for_context(
    context: Option<&AppContext>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        use uuid::Uuid;

        let tmp_file = TemporaryScreenshotFile::new(std::env::temp_dir().join(format!(
            "openhuman_screen_intelligence_{}.png",
            Uuid::new_v4()
        )));

        let capture_mode = capture_mode_for_context(context);
        log_capture_mode_decision(context, &capture_mode);

        let mut cmd = std::process::Command::new("screencapture");
        cmd.arg("-x").arg("-t").arg("png");

        if capture_mode == CaptureMode::Windowed {
            let b = &context
                .and_then(|ctx| ctx.bounds)
                .expect("windowed capture requires bounds");
            let rect = format!("{},{},{},{}", b.x, b.y, b.width, b.height);
            cmd.arg("-R").arg(&rect);
        } else {
            tracing::debug!("[accessibility] capture mode=fullscreen (primary display)");
        }

        cmd.arg(tmp_file.path());

        let output = cmd
            .output()
            .map_err(|e| format!("screencapture failed to start: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let permissions = detect_permissions();
            tracing::debug!(
                "[accessibility] screencapture failed status={:?} stderr={:?} screen_recording={:?}",
                output.status.code(),
                stderr,
                permissions.screen_recording
            );
            if permissions.screen_recording != PermissionState::Granted {
                return Err("screen recording permission is not granted".to_string());
            }
            if stderr.is_empty() {
                return Err(
                    "screen capture failed: screencapture returned non-zero status".to_string(),
                );
            }
            return Err(format!("screen capture failed: {}", stderr));
        }

        let bytes = std::fs::read(tmp_file.path())
            .map_err(|e| format!("failed to read captured screenshot: {e}"))?;
        tracing::debug!(
            "[accessibility] captured {} bytes (mode={})",
            bytes.len(),
            capture_mode
        );

        if let Some(width) = downscale_width_for_capture(bytes.len(), &capture_mode) {
            tracing::debug!(
                "[accessibility] {} capture {} bytes exceeds limit, retrying downscale width={}",
                capture_mode,
                bytes.len(),
                width
            );
            let sips_output = std::process::Command::new("sips")
                .arg("--resampleWidth")
                .arg(width)
                .arg(tmp_file.path())
                .output();
            match sips_output {
                Ok(output) if output.status.success() => {
                    let resized = match std::fs::read(tmp_file.path()) {
                        Ok(resized) => resized,
                        Err(e) => return Err(format!("failed to read resized screenshot: {e}")),
                    };
                    tracing::debug!("[accessibility] resized to {} bytes", resized.len());
                    if resized.len() > MAX_SCREENSHOT_BYTES {
                        return Err(
                            "captured screenshot exceeds size limit after downscale".to_string()
                        );
                    }
                    let encoded = BASE64_STANDARD.encode(resized);
                    return Ok(format!("data:image/png;base64,{encoded}"));
                }
                Ok(output) => {
                    tracing::debug!(
                        "[accessibility] sips failed status={:?} stderr={:?}",
                        output.status.code(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    );
                    return Err(
                        "captured screenshot exceeds size limit and downscale failed".to_string(),
                    );
                }
                Err(e) => {
                    tracing::debug!("[accessibility] sips not available: {e}");
                    return Err("captured screenshot exceeds size limit".to_string());
                }
            }
        }

        if bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err("captured screenshot exceeds size limit".to_string());
        }

        let encoded = BASE64_STANDARD.encode(bytes);
        Ok(format!("data:image/png;base64,{encoded}"))
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = context;
        Err("screen capture is unsupported on this platform".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::accessibility::ElementBounds;

    #[test]
    fn capture_mode_uses_window_bounds_when_positive() {
        let context = AppContext {
            app_name: Some("Code".to_string()),
            window_title: Some("main.rs".to_string()),
            bounds: Some(ElementBounds {
                x: 10,
                y: 20,
                width: 1440,
                height: 900,
            }),
        };

        assert_eq!(
            capture_mode_for_context(Some(&context)),
            CaptureMode::Windowed
        );
    }

    #[test]
    fn capture_mode_falls_back_to_fullscreen_for_missing_or_invalid_bounds() {
        let invalid_context = AppContext {
            app_name: Some("Finder".to_string()),
            window_title: Some("Desktop".to_string()),
            bounds: Some(ElementBounds {
                x: 0,
                y: 0,
                width: 0,
                height: 900,
            }),
        };

        assert_eq!(
            capture_mode_for_context(Some(&invalid_context)),
            CaptureMode::Fullscreen
        );
        assert_eq!(capture_mode_for_context(None), CaptureMode::Fullscreen);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn oversized_windowed_capture_is_eligible_for_downscale_retry() {
        assert_eq!(
            downscale_width_for_capture(MAX_SCREENSHOT_BYTES + 1, &CaptureMode::Windowed),
            Some(SCREENSHOT_DOWNSCALE_WIDTH)
        );
        assert_eq!(
            downscale_width_for_capture(MAX_SCREENSHOT_BYTES + 1, &CaptureMode::Fullscreen),
            Some(SCREENSHOT_DOWNSCALE_WIDTH)
        );
        assert_eq!(
            downscale_width_for_capture(MAX_SCREENSHOT_BYTES, &CaptureMode::Windowed),
            None
        );
    }
}
