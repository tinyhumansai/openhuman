//! Shell-side runtime toggle for webview-originated OS notifications.
//!
//! The embedded webviews (Slack, Gmail, Discord, …) can fire native OS
//! notifications via the CEF IPC hook in `webview_accounts`. This domain
//! owns the on/off switch. Default is ON so embedded SaaS webviews like
//! Slack behave like a normal browser session and surface native
//! notifications immediately after connection.
//!
//! State lives in the Tauri shell rather than the core sidecar so the
//! settings UI can flip it without a JSON-RPC round-trip. Persistence is
//! frontend-side (Redux/localStorage) — on boot the React side reads its
//! persisted value and pushes it down via `notification_settings_set`.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

/// Tauri-managed state holding the current feature-flag value.
///
/// Wrapped in an `AtomicBool` so reads from the CEF notification
/// callback (which runs on a CEF thread, not the Tauri runtime thread)
/// stay lock-free.
pub struct NotificationSettingsState {
    enabled: AtomicBool,
}

impl NotificationSettingsState {
    /// Construct the initial state. Embedded webview notifications default
    /// to **enabled** so provider permission grant immediately results in
    /// visible OS toasts unless the user later opts into DND/muting.
    pub fn new() -> Self {
        Self {
            enabled: AtomicBool::new(true),
        }
    }

    /// Current feature-flag value.
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    /// Update the feature-flag value. Returns the previous value so
    /// callers can log a single line about the transition if they want.
    pub fn set_enabled(&self, value: bool) -> bool {
        self.enabled.swap(value, Ordering::Relaxed)
    }
}

impl Default for NotificationSettingsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Payload returned to the frontend.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct NotificationSettingsPayload {
    pub enabled: bool,
}

/// Read the current notification feature-flag value.
#[tauri::command]
pub fn notification_settings_get(
    state: tauri::State<'_, NotificationSettingsState>,
) -> NotificationSettingsPayload {
    NotificationSettingsPayload {
        enabled: state.enabled(),
    }
}

/// Update the current notification feature-flag value. Returns the new
/// value so the caller can round-trip confirm.
#[tauri::command]
pub fn notification_settings_set(
    state: tauri::State<'_, NotificationSettingsState>,
    enabled: bool,
) -> NotificationSettingsPayload {
    let prev = state.set_enabled(enabled);
    log::info!(
        "[notify-settings] feature-flag transition enabled={} (was {})",
        enabled,
        prev
    );
    NotificationSettingsPayload { enabled }
}
