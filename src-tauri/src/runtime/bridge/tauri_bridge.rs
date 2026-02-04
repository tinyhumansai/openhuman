//! Platform and Tauri bridge for skills.
//!
//! Provides platform detection and native OS features like notifications.

/// Get the current platform name.
pub fn get_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "ios") {
        "ios"
    } else {
        "unknown"
    }
}

/// Send a native OS notification (desktop only).
#[cfg(desktop)]
pub fn send_notification(
    app_handle: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<(), String> {
    use tauri_plugin_notification::NotificationExt;
    app_handle
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| format!("Notification failed: {e}"))
}

/// Stub for mobile platforms where desktop notifications aren't available.
#[cfg(not(desktop))]
pub fn send_notification(
    _app_handle: &tauri::AppHandle,
    _title: &str,
    _body: &str,
) -> Result<(), String> {
    Err("Notifications not available on this platform".to_string())
}

/// Whitelisted environment values exposed to skills via `platform.env(key)`.
/// Skills should never hardcode host-specific URLs or secrets; use this instead.
pub fn get_skill_env(key: &str) -> Option<String> {
    match key {
        "BACKEND_URL" => Some(crate::utils::config::get_backend_url()),
        "PLATFORM" => Some(get_platform().to_string()),
        _ => None,
    }
}
