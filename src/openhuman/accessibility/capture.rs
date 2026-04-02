//! Timestamp helper and screen capture via platform-native tools.

use super::types::AppContext;

/// Maximum screenshot size in bytes before downscaling is attempted.
pub const MAX_SCREENSHOT_BYTES: usize = 1_500_000;

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

pub fn capture_screen_image_ref_for_context(
    context: Option<&AppContext>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
        use uuid::Uuid;

        let tmp_path = std::env::temp_dir().join(format!(
            "openhuman_screen_intelligence_{}.png",
            Uuid::new_v4()
        ));

        let bounds = context.and_then(|ctx| ctx.bounds);

        // Determine capture mode: windowed if we have valid bounds, fullscreen otherwise.
        let capture_mode = match &bounds {
            Some(b) if b.width > 0 && b.height > 0 => CaptureMode::Windowed,
            Some(b) => {
                tracing::debug!(
                    "[accessibility] invalid bounds ({}x{}), falling back to fullscreen",
                    b.width,
                    b.height
                );
                CaptureMode::Fullscreen
            }
            None => {
                tracing::debug!(
                    "[accessibility] no window bounds available, falling back to fullscreen"
                );
                CaptureMode::Fullscreen
            }
        };

        let mut cmd = std::process::Command::new("screencapture");
        cmd.arg("-x").arg("-t").arg("png");

        if capture_mode == CaptureMode::Windowed {
            let b = bounds.as_ref().unwrap();
            let rect = format!("{},{},{},{}", b.x, b.y, b.width, b.height);
            tracing::debug!(
                "[accessibility] capture mode=windowed rect={rect} app={:?}",
                context.and_then(|c| c.app_name.as_deref())
            );
            cmd.arg("-R").arg(&rect);
        } else {
            tracing::debug!("[accessibility] capture mode=fullscreen (primary display)");
        }

        cmd.arg(&tmp_path);

        let status = cmd
            .status()
            .map_err(|e| format!("screencapture failed to start: {e}"))?;
        if !status.success() {
            tracing::debug!(
                "[accessibility] screencapture exited with status: {:?}",
                status.code()
            );
            return Err("screencapture returned non-zero status".to_string());
        }

        let bytes =
            std::fs::read(&tmp_path).map_err(|e| format!("failed to read screenshot: {e}"))?;
        tracing::debug!(
            "[accessibility] captured {} bytes (mode={})",
            bytes.len(),
            capture_mode
        );

        // If fullscreen capture is too large, try downscaling with sips.
        if bytes.len() > MAX_SCREENSHOT_BYTES && capture_mode == CaptureMode::Fullscreen {
            tracing::debug!(
                "[accessibility] fullscreen capture {} bytes exceeds limit, downscaling with sips",
                bytes.len()
            );
            let sips_status = std::process::Command::new("sips")
                .arg("--resampleWidth")
                .arg("1280")
                .arg(&tmp_path)
                .status();
            match sips_status {
                Ok(s) if s.success() => {
                    let resized = match std::fs::read(&tmp_path) {
                        Ok(bytes) => bytes,
                        Err(e) => {
                            let _ = std::fs::remove_file(&tmp_path);
                            return Err(format!("failed to read resized screenshot: {e}"));
                        }
                    };
                    let _ = std::fs::remove_file(&tmp_path);
                    tracing::debug!("[accessibility] resized to {} bytes", resized.len());
                    if resized.len() > MAX_SCREENSHOT_BYTES {
                        return Err(
                            "captured screenshot exceeds size limit after downscale".to_string()
                        );
                    }
                    let encoded = BASE64_STANDARD.encode(resized);
                    return Ok(format!("data:image/png;base64,{encoded}"));
                }
                Ok(s) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    tracing::debug!("[accessibility] sips failed with status: {:?}", s.code());
                    return Err(
                        "captured screenshot exceeds size limit and downscale failed".to_string(),
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    tracing::debug!("[accessibility] sips not available: {e}");
                    return Err("captured screenshot exceeds size limit".to_string());
                }
            }
        }

        let _ = std::fs::remove_file(&tmp_path);

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
