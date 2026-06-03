//! Borderless always-on-top PTT overlay window. STUB — implemented in T6.

use tauri::{AppHandle, Runtime};

pub(crate) fn ensure_window<R: Runtime>(_app: &AppHandle<R>) -> Result<(), String> {
    // T6 will replace this with the real lazy create.
    Ok(())
}

pub(crate) fn destroy_window<R: Runtime>(_app: &AppHandle<R>) {
    // T6 will replace this with the real destroy.
}

#[tauri::command]
pub(crate) async fn show_ptt_overlay<R: Runtime>(
    _app: AppHandle<R>,
    _active: bool,
    _session_id: u64,
) -> Result<(), String> {
    // T6 will replace this with the real show/hide.
    Ok(())
}
