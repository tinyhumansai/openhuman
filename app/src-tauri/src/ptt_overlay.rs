//! Borderless always-on-top PTT overlay window.
//!
//! Lazy-created on the first `register_ptt_hotkey` call (so the window is
//! ready when the user hits the key for the first time), and destroyed by
//! `unregister_ptt_hotkey`. The window's contents are rendered by the React
//! route `/ptt-overlay` (see `app/src/pages/PttOverlayPage.tsx`).
//!
//! Cross-platform note: `focus(false)` ensures the window never steals focus
//! from the user's active app. `skip_taskbar(true)` keeps it out of the
//! Windows taskbar / macOS dock. `visible_on_all_workspaces(true)` makes it
//! follow the user across macOS Spaces. DXGI exclusive-fullscreen on Windows
//! still suppresses the overlay — documented in the settings panel as a
//! limitation; chime audio remains the fallback signal.

use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

const OVERLAY_LABEL: &str = "ptt-overlay";

/// Ensure the overlay window exists. Idempotent — if the window already
/// exists, returns Ok without recreating it.
pub(crate) fn ensure_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    if app.get_webview_window(OVERLAY_LABEL).is_some() {
        return Ok(());
    }
    let url = WebviewUrl::App("index.html#/ptt-overlay".into());
    let mut builder = WebviewWindowBuilder::new(app, OVERLAY_LABEL, url)
        .title("OpenHuman Push-to-Talk")
        .inner_size(160.0, 56.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .resizable(false)
        // NOTE: .shadow(false) is a no-op under the project's CEF runtime
        // (tauri-runtime-cef has a TODO stub); harmless but won't actually
        // suppress the OS shadow until CEF wires it through.
        .shadow(false)
        .visible(false);

    #[cfg(target_os = "macos")]
    {
        builder = builder
            .visible_on_all_workspaces(true)
            .accept_first_mouse(false);
    }

    let _window = builder
        .build()
        .map_err(|e| format!("create ptt overlay window: {e}"))?;
    log::info!("[ptt-overlay] window created (label={OVERLAY_LABEL})");
    Ok(())
}

/// Destroy the overlay window if it exists.
pub(crate) fn destroy_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(OVERLAY_LABEL) {
        if let Err(e) = w.destroy() {
            log::warn!("[ptt-overlay] destroy failed: {e}");
        } else {
            log::info!("[ptt-overlay] window destroyed");
        }
    }
}

/// Show or hide the overlay. Emits `ptt-overlay://active` for the in-window
/// React tree to drive its pulsing-dot animation.
#[tauri::command]
pub(crate) async fn show_ptt_overlay<R: Runtime>(
    app: AppHandle<R>,
    active: bool,
    session_id: u64,
) -> Result<(), String> {
    let window = app.get_webview_window(OVERLAY_LABEL).ok_or_else(|| {
        "[ptt-overlay] window not ready (register_ptt_hotkey must succeed before show_ptt_overlay)"
            .to_string()
    })?;

    if active {
        window.show().map_err(|e| format!("show overlay: {e}"))?;
    } else {
        window.hide().map_err(|e| format!("hide overlay: {e}"))?;
    }

    if let Err(e) = window.emit(
        "ptt-overlay://active",
        serde_json::json!({
            "active": active,
            "session_id": session_id,
        }),
    ) {
        log::warn!("[ptt-overlay] emit active failed: {e}");
    }

    Ok(())
}
