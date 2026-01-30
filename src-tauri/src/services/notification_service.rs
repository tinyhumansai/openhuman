use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

/// Service for managing native notifications
pub struct NotificationService;

impl NotificationService {
    /// Show a simple notification
    pub fn show(app: &AppHandle, title: &str, body: &str) -> Result<(), String> {
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|e| format!("Failed to show notification: {}", e))
    }

    /// Show a notification with an icon
    pub fn show_with_icon(
        app: &AppHandle,
        title: &str,
        body: &str,
        icon: &str,
    ) -> Result<(), String> {
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .icon(icon)
            .show()
            .map_err(|e| format!("Failed to show notification: {}", e))
    }

    /// Show a notification for a new message
    pub fn show_message_notification(
        app: &AppHandle,
        sender: &str,
        message: &str,
    ) -> Result<(), String> {
        Self::show(app, &format!("New message from {}", sender), message)
    }

    /// Check if notifications are permitted
    pub fn is_permission_granted(app: &AppHandle) -> Result<bool, String> {
        app.notification()
            .permission_state()
            .map(|state| state == tauri_plugin_notification::PermissionState::Granted)
            .map_err(|e| format!("Failed to check permission: {}", e))
    }

    /// Request notification permission
    pub fn request_permission(app: &AppHandle) -> Result<bool, String> {
        app.notification()
            .request_permission()
            .map(|state| state == tauri_plugin_notification::PermissionState::Granted)
            .map_err(|e| format!("Failed to request permission: {}", e))
    }
}
