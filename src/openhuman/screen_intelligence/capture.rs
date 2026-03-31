use chrono::Utc;

#[cfg(target_os = "macos")]
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
#[cfg(target_os = "macos")]
use uuid::Uuid;

use super::context::{AppContext, WindowBounds};
#[cfg(target_os = "macos")]
use super::limits::MAX_SCREENSHOT_BYTES;

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Capture mode used for a screenshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CaptureMode {
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

pub(crate) fn capture_screen_image_ref_for_context(
    context: Option<&AppContext>,
) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        let tmp_path = std::env::temp_dir().join(format!(
            "openhuman_screen_intelligence_{}.png",
            Uuid::new_v4()
        ));

        let bounds = context.and_then(|ctx| ctx.bounds.clone());

        // Determine capture mode: windowed if we have valid bounds, fullscreen otherwise.
        let capture_mode = match &bounds {
            Some(b) if b.width > 0 && b.height > 0 => CaptureMode::Windowed,
            Some(b) => {
                tracing::debug!(
                    "[screen_intelligence] invalid bounds ({}x{}), falling back to fullscreen",
                    b.width,
                    b.height
                );
                CaptureMode::Fullscreen
            }
            None => {
                tracing::debug!(
                    "[screen_intelligence] no window bounds available, falling back to fullscreen"
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
                "[screen_intelligence] capture mode=windowed rect={rect} app={:?}",
                context.and_then(|c| c.app_name.as_deref())
            );
            cmd.arg("-R").arg(&rect);
        } else {
            tracing::debug!("[screen_intelligence] capture mode=fullscreen (primary display)");
        }

        cmd.arg(&tmp_path);

        let status = cmd
            .status()
            .map_err(|e| format!("screencapture failed to start: {e}"))?;
        if !status.success() {
            tracing::debug!(
                "[screen_intelligence] screencapture exited with status: {:?}",
                status.code()
            );
            return Err("screencapture returned non-zero status".to_string());
        }

        let bytes =
            std::fs::read(&tmp_path).map_err(|e| format!("failed to read screenshot: {e}"))?;
        tracing::debug!(
            "[screen_intelligence] captured {} bytes (mode={})",
            bytes.len(),
            capture_mode
        );

        // If fullscreen capture is too large, try downscaling with sips.
        if bytes.len() > MAX_SCREENSHOT_BYTES && capture_mode == CaptureMode::Fullscreen {
            tracing::debug!(
                "[screen_intelligence] fullscreen capture {} bytes exceeds limit, downscaling with sips",
                bytes.len()
            );
            let sips_status = std::process::Command::new("sips")
                .arg("--resampleWidth")
                .arg("1280")
                .arg(&tmp_path)
                .status();
            match sips_status {
                Ok(s) if s.success() => {
                    let resized = std::fs::read(&tmp_path)
                        .map_err(|e| format!("failed to read resized screenshot: {e}"))?;
                    let _ = std::fs::remove_file(&tmp_path);
                    tracing::debug!("[screen_intelligence] resized to {} bytes", resized.len());
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
                    tracing::debug!(
                        "[screen_intelligence] sips failed with status: {:?}",
                        s.code()
                    );
                    return Err(
                        "captured screenshot exceeds size limit and downscale failed".to_string(),
                    );
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&tmp_path);
                    tracing::debug!("[screen_intelligence] sips not available: {e}");
                    return Err("captured screenshot exceeds size limit".to_string());
                }
            }
        }

        let _ = std::fs::remove_file(&tmp_path);

        if bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err("captured screenshot exceeds size limit".to_string());
        }

        let encoded = BASE64_STANDARD.encode(bytes);
        return Ok(format!("data:image/png;base64,{encoded}"));
    }

    #[cfg(not(target_os = "macos"))]
    {
        let _ = context;
        Err("screen capture is unsupported on this platform".to_string())
    }
}

/// Parse the raw stdout from the AppleScript foreground-context query.
///
/// Expected format: 6 lines — app_name, window_title, x, y, width, height.
/// This is a pure function, fully testable without macOS.
pub(crate) fn parse_foreground_output(stdout: &str) -> Option<AppContext> {
    let mut lines = stdout.lines();
    let app = lines.next().map(|s| s.trim().to_string());
    let title = lines.next().map(|s| s.trim().to_string());
    let x = lines.next().and_then(|s| s.trim().parse::<i32>().ok());
    let y = lines.next().and_then(|s| s.trim().parse::<i32>().ok());
    let width = lines.next().and_then(|s| s.trim().parse::<i32>().ok());
    let height = lines.next().and_then(|s| s.trim().parse::<i32>().ok());

    let bounds = match (x, y, width, height) {
        (Some(x), Some(y), Some(width), Some(height)) if width > 0 && height > 0 => {
            Some(WindowBounds {
                x,
                y,
                width,
                height,
            })
        }
        _ => None,
    };

    Some(AppContext {
        app_name: app.filter(|s| !s.is_empty()),
        window_title: title.filter(|s| !s.is_empty()),
        bounds,
    })
}

#[cfg(target_os = "macos")]
pub(crate) fn foreground_context() -> Option<AppContext> {
    let script = r#"
      tell application "System Events"
        set frontApp to name of first application process whose frontmost is true
        set frontWindow to ""
        set windowX to ""
        set windowY to ""
        set windowW to ""
        set windowH to ""
        try
          tell process frontApp
            if (count of windows) > 0 then
              set w to front window
              set frontWindow to name of w
              set p to position of w
              set s to size of w
              set windowX to item 1 of p as text
              set windowY to item 2 of p as text
              set windowW to item 1 of s as text
              set windowH to item 2 of s as text
            end if
          end tell
        end try
        return frontApp & "\n" & frontWindow & "\n" & windowX & "\n" & windowY & "\n" & windowW & "\n" & windowH
      end tell
    "#;

    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(script)
        .output()
        .ok()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::debug!(
            "[screen_intelligence] osascript failed: status={:?} stderr={}",
            output.status.code(),
            stderr.trim()
        );
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let result = parse_foreground_output(&text);
    tracing::debug!(
        "[screen_intelligence] foreground_context: app={:?} bounds_present={}",
        result.as_ref().and_then(|c| c.app_name.as_deref()),
        result.as_ref().map(|c| c.bounds.is_some()).unwrap_or(false)
    );
    result
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn foreground_context() -> Option<AppContext> {
    None
}
