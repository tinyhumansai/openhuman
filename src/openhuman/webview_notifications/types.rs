//! Shared wire types for webview-originated notifications.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Payload emitted from the Tauri shell when a webview renderer fires a
/// `window.Notification`. Carried verbatim to the React side over the
/// `webview-notification:fired` Tauri event so the UI can bump unread
/// counts, show its own in-app toast, and route a subsequent click back
/// to the right embedded webview via Redux.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WebviewNotificationEvent {
    /// Stable account id from the Redux `accounts` slice (persisted).
    pub account_id: String,
    /// Provider id, e.g. `slack`, `gmail`, `discord`.
    pub provider: String,
    /// OS-visible title (already `OpenHuman:`-prefixed by `format_title`).
    pub title: String,
    /// OS-visible body. Empty string when the page didn't set one.
    pub body: String,
    /// Optional renderer-supplied `tag` for native dedup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

/// Runtime on/off toggle for the feature. Defaults to **disabled** —
/// v1 ships the plumbing but requires an explicit opt-in so the
/// release doesn't suddenly start firing OS toasts for every
/// background DM in an idle Slack tab.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct NotificationSettings {
    pub enabled: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self { enabled: false }
    }
}
