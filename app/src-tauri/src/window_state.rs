//! Persistence of main-window position + size across restarts.
//!
//! `app.restart()` (used by #900's identity-flip flow) spawns a fresh
//! process, so the new window doesn't inherit anything from the old one.
//! Without us re-applying state, every login-driven respawn snaps the
//! window back to the default initial size in the center of the primary
//! display — even when the user had it on an external monitor or had
//! resized it.
//!
//! This module persists a tiny TOML record at
//! `<openhuman_dir>/window_state.toml` capturing the outer position and
//! outer size of the main window in physical pixels. On launch the
//! record is read and applied before the window is shown. On restart we
//! save first, hide the window, then call `app.restart()`.
//!
//! Saved state is best-effort: read errors, missing file, off-screen
//! positions, and non-existent monitors all fall back to the default
//! centered window so we never trap the window where the user can't
//! reach it.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

use crate::cef_profile;

const STATE_FILE: &str = "window_state.toml";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowState {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn state_path() -> Option<PathBuf> {
    cef_profile::default_root_openhuman_dir()
        .ok()
        .map(|root| root.join(STATE_FILE))
}

/// Capture the main window's outer geometry and write it to disk.
///
/// Called from `restart_app` immediately before `app.restart()` so the
/// next process can land the new window where the user left it.
pub fn save_main<R: Runtime>(window: &WebviewWindow<R>) {
    let Ok(pos) = window.outer_position() else {
        log::warn!("[window-state] outer_position unavailable; skip save");
        return;
    };
    let Ok(size) = window.outer_size() else {
        log::warn!("[window-state] outer_size unavailable; skip save");
        return;
    };
    let state = WindowState {
        x: pos.x,
        y: pos.y,
        width: size.width,
        height: size.height,
    };
    let Some(path) = state_path() else {
        log::warn!("[window-state] no path available; skip save");
        return;
    };
    if let Some(parent) = path.parent() {
        if let Err(err) = std::fs::create_dir_all(parent) {
            log::warn!(
                "[window-state] mkdir {} failed: {}; skip save",
                parent.display(),
                err
            );
            return;
        }
    }
    let raw = match toml::to_string_pretty(&state) {
        Ok(r) => r,
        Err(err) => {
            log::warn!("[window-state] serialize failed: {err}; skip save");
            return;
        }
    };
    if let Err(err) = std::fs::write(&path, raw) {
        log::warn!("[window-state] write {} failed: {err}", path.display());
    } else {
        log::info!(
            "[window-state] saved geometry x={} y={} w={} h={}",
            state.x,
            state.y,
            state.width,
            state.height
        );
    }
}

/// Read the saved geometry (if any) and apply it to the main window.
///
/// Returns `true` when saved geometry was applied. Returns `false` when
/// no saved file exists, the file is malformed, or the saved position
/// falls outside every currently-attached monitor (e.g. the user
/// undocked an external display); the caller is then expected to fall
/// back to a centered default so we never strand the window off-screen.
pub fn restore_main<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    let Some(path) = state_path() else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return false;
    };
    let state: WindowState = match toml::from_str(&raw) {
        Ok(s) => s,
        Err(err) => {
            log::warn!(
                "[window-state] parse {} failed: {err}; using default placement",
                path.display()
            );
            return false;
        }
    };

    if !position_visible_on_any_monitor(window, state.x, state.y, state.width, state.height) {
        log::info!(
            "[window-state] saved position x={} y={} not on any monitor; falling back to centered default",
            state.x,
            state.y
        );
        return false;
    }

    if let Err(err) = window.set_size(PhysicalSize::new(state.width, state.height)) {
        log::warn!("[window-state] set_size failed: {err}");
    }
    if let Err(err) = window.set_position(PhysicalPosition::new(state.x, state.y)) {
        log::warn!("[window-state] set_position failed: {err}");
        return false;
    }
    log::info!(
        "[window-state] restored geometry x={} y={} w={} h={}",
        state.x,
        state.y,
        state.width,
        state.height
    );
    true
}

/// Center the main window on the primary display (or its current monitor
/// if `current_monitor` resolves) when no saved state applied.
pub fn center_main<R: Runtime>(window: &WebviewWindow<R>) {
    let Ok(Some(monitor)) = window
        .primary_monitor()
        .or_else(|_| window.current_monitor())
    else {
        let _ = window.center();
        return;
    };
    let Ok(size) = window.outer_size() else {
        let _ = window.center();
        return;
    };
    let mon_pos = monitor.position();
    let mon_size = monitor.size();
    let x = mon_pos.x + (mon_size.width as i32 - size.width as i32) / 2;
    let y = mon_pos.y + (mon_size.height as i32 - size.height as i32) / 2;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn position_visible_on_any_monitor<R: Runtime>(
    window: &WebviewWindow<R>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
) -> bool {
    let Ok(monitors) = window.available_monitors() else {
        return false;
    };
    // Treat the window as on-screen if at least a 100x100 px patch of it
    // overlaps any attached monitor.
    let win_right = x.saturating_add(width as i32);
    let win_bottom = y.saturating_add(height as i32);
    monitors.iter().any(|m| {
        let pos = m.position();
        let size = m.size();
        let mon_right = pos.x.saturating_add(size.width as i32);
        let mon_bottom = pos.y.saturating_add(size.height as i32);
        let overlap_w = (win_right.min(mon_right) - x.max(pos.x)).max(0);
        let overlap_h = (win_bottom.min(mon_bottom) - y.max(pos.y)).max(0);
        overlap_w >= 100 && overlap_h >= 100
    })
}
