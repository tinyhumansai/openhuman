//! Native OS notification commands.
//!
//! Single source of truth for the in-app "Send test notification" flow and
//! the background service that surfaces agent / system events as banners
//! when the window isn't focused.
//!
//! Why this module exists rather than calling `tauri-plugin-notification`
//! from the frontend directly:
//!
//! * The bundled plugin's `permission_state()` and `request_permission()`
//!   are hardcoded to `Granted` (see
//!   `vendor/tauri-plugin-notification/src/desktop.rs`), so a frontend
//!   permission gate built on `plugin:notification|is_permission_granted`
//!   reports success even when macOS has notifications disabled for the
//!   bundle — which is the root cause of issue #1152.
//! * The plugin's `.show()` spawns the actual `notify-rust` call on a
//!   background task and discards the inner result, so any delivery
//!   failure is swallowed and the UI falsely reports "sent."
//!
//! On macOS we drive `UNUserNotificationCenter` directly via `objc2` so
//! both the authorization check and the dispatch are real, with delivery
//! errors propagated through the completion handler. On Linux/Windows the
//! plugin path is sufficient and we delegate to it.
use tauri::AppHandle;

#[cfg(not(target_os = "macos"))]
use tauri_plugin_notification::NotificationExt;

use crate::AppRuntime;

/// Tauri command: report the current OS notification authorization state.
///
/// Returns one of: `granted`, `denied`, `not_determined`, `provisional`,
/// `ephemeral`, `unknown`. Non-macOS targets always return `granted`
/// because there is no equivalent OS-level prompt to gate on.
#[tauri::command]
pub fn notification_permission_state() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        return macos::permission_state();
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("granted".to_string())
    }
}

/// Tauri command: trigger the OS-level permission prompt and return the
/// resulting authorization state (`granted` or `denied` on macOS).
#[tauri::command]
pub fn notification_permission_request() -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        return macos::request_permission();
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok("granted".to_string())
    }
}

/// Tauri command: fire a native OS notification.
///
/// On macOS, fails fast if notification permission is not actually granted
/// and waits for the `addNotificationRequest:withCompletionHandler:`
/// completion before returning, so a successful return means the system
/// accepted the request — not just that a `.show()` future was spawned.
#[tauri::command]
pub fn show_native_notification(
    app: AppHandle<AppRuntime>,
    title: String,
    body: String,
    tag: Option<String>,
) -> Result<(), String> {
    log::debug!(
        "[notify] show_native_notification title_chars={} body_chars={} tag={:?}",
        title.len(),
        body.len(),
        tag
    );

    #[cfg(target_os = "macos")]
    {
        let _ = app;
        macos::show(title, body, tag)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let mut builder = app.notification().builder().title(&title);
        if !body.is_empty() {
            builder = builder.body(&body);
        }
        builder
            .show()
            .map_err(|e| format!("notification show failed: {e}"))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ptr::NonNull;
    use std::sync::mpsc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::{NSError, NSString};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent,
        UNNotificationRequest, UNNotificationSettings, UNNotificationSound,
        UNUserNotificationCenter,
    };

    /// Read authorization status synchronously by blocking on
    /// `getNotificationSettingsWithCompletionHandler:`.
    pub(super) fn permission_state() -> Result<String, String> {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (tx, rx) = mpsc::channel::<String>();
        let completion = RcBlock::new(move |settings: NonNull<UNNotificationSettings>| {
            let status = unsafe { settings.as_ref().authorizationStatus() };
            let _ = tx.send(status_to_str(status).to_string());
        });
        center.getNotificationSettingsWithCompletionHandler(&completion);
        rx.recv_timeout(Duration::from_secs(2))
            .map_err(|_| "timed out waiting for macOS notification settings".to_string())
    }

    /// Trigger the OS prompt for notification authorization. Returns
    /// `granted` if the user accepted (or had previously accepted),
    /// `denied` otherwise.
    pub(super) fn request_permission() -> Result<String, String> {
        let center = UNUserNotificationCenter::currentNotificationCenter();
        let (tx, rx) = mpsc::channel::<bool>();
        let options = UNAuthorizationOptions::Alert
            | UNAuthorizationOptions::Badge
            | UNAuthorizationOptions::Sound;
        let completion = RcBlock::new(move |granted: Bool, _error: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        center.requestAuthorizationWithOptions_completionHandler(options, &completion);
        let granted = rx
            .recv_timeout(Duration::from_secs(5))
            .map_err(|_| "timed out waiting for macOS permission prompt result".to_string())?;
        Ok(if granted { "granted" } else { "denied" }.to_string())
    }

    /// Build a `UNNotificationRequest` and submit it. Re-checks
    /// authorization first so we never call `addNotificationRequest:` on
    /// a denied/not-determined state — the API would silently accept the
    /// call but the OS would drop the banner, which is exactly the
    /// "reports success but nothing appears" failure mode of #1152.
    pub(super) fn show(title: String, body: String, tag: Option<String>) -> Result<(), String> {
        let state = permission_state()?;
        if !is_granted(&state) {
            log::warn!("[notify] show aborted: permission state={state}");
            return Err(format!(
                "notification permission not granted (state: {state})"
            ));
        }

        let center = UNUserNotificationCenter::currentNotificationCenter();
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str(&title));
        if !body.is_empty() {
            content.setBody(&NSString::from_str(&body));
        }
        let default_sound = UNNotificationSound::defaultSound();
        content.setSound(Some(&default_sound));

        // UN dedupes pending requests by identifier, so a unique value per
        // call ensures repeated taps of "Send test notification" each
        // fire a fresh banner. Falls back to a timestamp when the caller
        // didn't supply a tag.
        let identifier_str = tag.unwrap_or_else(|| {
            format!(
                "openhuman.notify.{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_nanos())
                    .unwrap_or(0)
            )
        });
        let identifier = NSString::from_str(&identifier_str);

        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
            &identifier,
            &content,
            None,
        );

        let (tx, rx) = mpsc::channel::<Option<String>>();
        let completion = RcBlock::new(move |error: *mut NSError| {
            if error.is_null() {
                let _ = tx.send(None);
                return;
            }
            // SAFETY: UN guarantees `error` lives for the duration of the
            // completion callback when non-null.
            let message = unsafe { (*error).localizedDescription().to_string() };
            let _ = tx.send(Some(message));
        });

        center.addNotificationRequest_withCompletionHandler(&request, Some(&completion));

        match rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "timed out waiting for macOS notification dispatch".to_string())?
        {
            None => {
                log::debug!("[notify] macos add succeeded id={identifier_str}");
                Ok(())
            }
            Some(err) => {
                log::warn!("[notify] macos add failed: {err}");
                Err(format!("notification show failed: {err}"))
            }
        }
    }

    fn is_granted(state: &str) -> bool {
        matches!(state, "granted" | "provisional" | "ephemeral")
    }

    fn status_to_str(status: UNAuthorizationStatus) -> &'static str {
        if status == UNAuthorizationStatus::Authorized {
            "granted"
        } else if status == UNAuthorizationStatus::Denied {
            "denied"
        } else if status == UNAuthorizationStatus::NotDetermined {
            "not_determined"
        } else if status == UNAuthorizationStatus::Provisional {
            "provisional"
        } else if status == UNAuthorizationStatus::Ephemeral {
            "ephemeral"
        } else {
            "unknown"
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn is_granted_treats_authorized_variants_as_granted() {
            assert!(is_granted("granted"));
            assert!(is_granted("provisional"));
            assert!(is_granted("ephemeral"));
        }

        #[test]
        fn is_granted_rejects_unauthorized_states() {
            assert!(!is_granted("denied"));
            assert!(!is_granted("not_determined"));
            assert!(!is_granted("unknown"));
            assert!(!is_granted(""));
        }

        #[test]
        fn status_to_str_maps_known_statuses() {
            assert_eq!(status_to_str(UNAuthorizationStatus::Authorized), "granted");
            assert_eq!(status_to_str(UNAuthorizationStatus::Denied), "denied");
            assert_eq!(
                status_to_str(UNAuthorizationStatus::NotDetermined),
                "not_determined"
            );
            assert_eq!(
                status_to_str(UNAuthorizationStatus::Provisional),
                "provisional"
            );
            assert_eq!(status_to_str(UNAuthorizationStatus::Ephemeral), "ephemeral");
        }
    }
}
