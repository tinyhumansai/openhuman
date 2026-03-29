use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use chrono::Utc;
use uuid::Uuid;

use super::context::{AppContext, WindowBounds};
use super::limits::MAX_SCREENSHOT_BYTES;

pub(crate) fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
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

        let bounds = context
            .and_then(|ctx| ctx.bounds.clone())
            .ok_or_else(|| "active window bounds unavailable".to_string())?;
        if bounds.width <= 0 || bounds.height <= 0 {
            return Err("active window bounds are invalid".to_string());
        }

        let rect = format!(
            "{},{},{},{}",
            bounds.x, bounds.y, bounds.width, bounds.height
        );

        let status = std::process::Command::new("screencapture")
            .arg("-x")
            .arg("-t")
            .arg("png")
            .arg("-R")
            .arg(&rect)
            .arg(&tmp_path)
            .status()
            .map_err(|e| format!("screencapture failed to start: {e}"))?;
        if !status.success() {
            return Err("screencapture returned non-zero status".to_string());
        }
        let bytes =
            std::fs::read(&tmp_path).map_err(|e| format!("failed to read screenshot: {e}"))?;
        let _ = std::fs::remove_file(&tmp_path);
        if bytes.len() > MAX_SCREENSHOT_BYTES {
            return Err("captured screenshot exceeds size limit".to_string());
        }
        let encoded = BASE64_STANDARD.encode(bytes);
        return Ok(format!("data:image/png;base64,{encoded}"));
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err("screen capture is unsupported on this platform".to_string())
    }
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
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut lines = text.lines();
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

#[cfg(not(target_os = "macos"))]
pub(crate) fn foreground_context() -> Option<AppContext> {
    None
}
