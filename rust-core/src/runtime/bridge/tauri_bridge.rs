//! Platform and Tauri bridge for skills.
//!
//! Provides platform detection and native OS features like notifications.

/// Get the current platform name.
pub fn get_platform() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        "linux" => "linux",
        "android" => "android",
        "ios" => "ios",
        _ => "unknown",
    }
}

/// Send a native OS notification (desktop only).
pub fn send_notification(
    _app_handle: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<(), String> {
    if matches!(get_platform(), "android" | "ios") {
        return Err("Notifications not available on this platform".to_string());
    }
    log::info!("[runtime] notification requested: {title} - {body}");
    Ok(())
}

/// Whitelisted environment values exposed to skills via `platform.env(key)`.
/// Skills should never hardcode host-specific URLs or secrets; use this instead.
pub fn get_skill_env(key: &str) -> Option<String> {
    match key {
        "BACKEND_URL" => Some(
            std::env::var("VITE_BACKEND_URL")
                .ok()
                .filter(|url| !url.trim().is_empty())
                .unwrap_or_else(|| "http://localhost:5005".to_string()),
        ),
        "PLATFORM" => Some(get_platform().to_string()),
        _ => None,
    }
}
