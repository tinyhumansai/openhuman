//! Tauri commands for the desktop companion hotkey and activation.
//!
//! Mirrors the dictation hotkey pattern: managed state tracks registered
//! shortcuts, register/unregister commands handle lifecycle, and the
//! hotkey press emits a Tauri event for the frontend to consume.

use std::sync::Mutex;

use log::{debug, info, warn};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

use crate::dictation_hotkeys;
use crate::AppRuntime;

/// Tracks registered companion hotkey strings for unregistration.
pub(crate) struct CompanionHotkeyState(pub(crate) Mutex<Vec<String>>);

/// Register (or re-register) the global companion activation hotkey.
/// Emits `companion://activate` to all webviews when the shortcut is pressed.
#[tauri::command]
pub(crate) async fn register_companion_hotkey(
    app: AppHandle<AppRuntime>,
    shortcut: String,
) -> Result<(), String> {
    info!("[companion] register_companion_hotkey: shortcut={shortcut}");

    let old_shortcuts = {
        let state = app.state::<CompanionHotkeyState>();
        let guard = state.0.lock().unwrap();
        guard.clone()
    };

    let expanded = dictation_hotkeys::expand_dictation_shortcuts(&shortcut);
    if expanded.is_empty() {
        return Err("Shortcut cannot be empty".to_string());
    }
    info!("[companion] expanded shortcuts: {}", expanded.join(", "));

    let register_shortcut = |variant: &str| -> Result<(), String> {
        let app_clone = app.clone();
        app.global_shortcut()
            .on_shortcut(variant, move |_app, _sc, event| {
                if event.state == ShortcutState::Pressed {
                    debug!("[companion] hotkey pressed — emitting companion://activate");
                    if let Err(e) = app_clone.emit("companion://activate", ()) {
                        warn!("[companion] emit failed: {e}");
                    }
                }
            })
            .map_err(|e| format!("Failed to register shortcut '{variant}': {e}"))
    };

    // Unregister old shortcuts (with rollback on failure).
    let mut unregistered_old: Vec<String> = Vec::new();
    for old in &old_shortcuts {
        debug!("[companion] unregistering previous shortcut: {old}");
        if let Err(e) = app.global_shortcut().unregister(old.as_str()) {
            for restored in &unregistered_old {
                if let Err(err) = register_shortcut(restored.as_str()) {
                    warn!("[companion] rollback failed restoring '{restored}': {err}");
                }
            }
            return Err(format!(
                "Failed to unregister previous shortcut '{old}': {e}"
            ));
        }
        unregistered_old.push(old.clone());
    }

    // Register new shortcuts (with rollback on failure).
    let mut newly_registered: Vec<String> = Vec::new();
    for variant in &expanded {
        if let Err(err) = register_shortcut(variant.as_str()) {
            warn!("[companion] failed to register '{variant}': {err}");
            for reg in &newly_registered {
                if let Err(e) = app.global_shortcut().unregister(reg.as_str()) {
                    warn!("[companion] rollback unregister '{reg}' failed: {e}");
                }
            }
            for old in &old_shortcuts {
                if let Err(e) = register_shortcut(old.as_str()) {
                    warn!("[companion] rollback restoring old '{old}' failed: {e}");
                }
            }
            return Err(err);
        }
        newly_registered.push(variant.clone());
    }

    // Persist registered shortcuts.
    {
        let state = app.state::<CompanionHotkeyState>();
        let mut guard = state.0.lock().unwrap();
        *guard = expanded.clone();
    }

    info!("[companion] shortcuts registered: {}", expanded.join(", "));
    Ok(())
}

/// Unregister the global companion hotkey (if any).
#[tauri::command]
pub(crate) async fn unregister_companion_hotkey(app: AppHandle<AppRuntime>) -> Result<(), String> {
    info!("[companion] unregister_companion_hotkey: called");
    let state = app.state::<CompanionHotkeyState>();
    let mut guard = state.0.lock().unwrap();
    if guard.is_empty() {
        debug!("[companion] no shortcut registered — nothing to unregister");
    } else {
        // Clear the in-memory registry only after every OS unregister succeeds.
        // If we clear first and a later unregister fails, we leak a registered
        // shortcut with no record to retry.
        let old = guard.clone();
        for shortcut in &old {
            debug!("[companion] unregistering shortcut: {shortcut}");
            app.global_shortcut()
                .unregister(shortcut.as_str())
                .map_err(|e| {
                    warn!("[companion] failed to unregister '{shortcut}': {e}");
                    format!("Failed to unregister shortcut '{shortcut}': {e}")
                })?;
            info!("[companion] shortcut unregistered: {shortcut}");
        }
        guard.clear();
    }
    Ok(())
}

/// Programmatic companion activation (e.g. from a "Test" button in settings).
#[tauri::command]
pub(crate) async fn companion_activate(app: AppHandle<AppRuntime>) -> Result<(), String> {
    info!("[companion] companion_activate: called");
    app.emit("companion://activate", ())
        .map_err(|e| format!("Failed to emit companion://activate: {e}"))
}
