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
//!
//! Window geometry — both restored saved state and the default initial
//! size from `tauri.conf.json` — is always clamped to the active
//! monitor's **work area** (the screen minus OS chrome: macOS menu
//! bar + dock, Windows taskbar, Linux panels). This prevents the window
//! from opening taller than the screen and hiding the bottom navigation
//! on small or scaled displays — see issue #2282. We also re-clamp on
//! restore so a window saved on a large external display does not come
//! back oversized after the user undocks onto a small laptop screen.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

use crate::cef_profile;

const STATE_FILE: &str = "window_state.toml";

/// Smallest size we will ever shrink the window to. Below this the UI
/// becomes unusable (no room for the sidebar/chat layout at all), so
/// clamping refuses to go further even on tiny monitors. Physical
/// pixels — at 1× this is roughly the smallest viable phone-portrait
/// shape; at 2× retina it's effectively half that in logical pixels.
const MIN_WINDOW_WIDTH: u32 = 480;
const MIN_WINDOW_HEIGHT: u32 = 360;

/// Minimum overlap (px on each axis) between the saved window rect and a
/// monitor's work area for us to treat the window as "still on that
/// monitor". Matches the historical `position_visible_on_any_monitor`
/// threshold so disconnecting the external display still falls back to
/// the centered default instead of stranding the window off-screen.
const MIN_VISIBLE_OVERLAP_PX: i32 = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WindowState {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

/// A monitor's usable work area in physical pixels. Plain-data struct so
/// the geometry math in [`clamp_to_work_area`] / [`pick_monitor_for_window`]
/// can be unit-tested without a live Tauri runtime.
#[derive(Debug, Clone, Copy)]
struct WorkArea {
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
/// falls outside every currently-attached monitor's work area (e.g. the
/// user undocked an external display); the caller is then expected to
/// fall back to a centered default so we never strand the window
/// off-screen.
///
/// Even when the saved monitor is still attached, the restored size is
/// clamped to that monitor's work area (issue #2282) so a window saved
/// on a large external display does not come back taller/wider than the
/// laptop the user is currently on.
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

    let work_areas = collect_work_areas(window);
    if work_areas.is_empty() {
        log::warn!(
            "[window-state] no monitors reported; cannot validate saved geometry, using default"
        );
        return false;
    }

    let Some(monitor) =
        pick_monitor_for_window(state.x, state.y, state.width, state.height, &work_areas)
    else {
        log::info!(
            "[window-state] saved geometry x={} y={} w={} h={} not on any attached monitor's work area; falling back to centered default",
            state.x,
            state.y,
            state.width,
            state.height
        );
        return false;
    };

    let (x, y, width, height) =
        clamp_to_work_area(state.x, state.y, state.width, state.height, monitor);

    if let Err(err) = window.set_size(PhysicalSize::new(width, height)) {
        log::warn!("[window-state] set_size failed: {err}");
    }
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("[window-state] set_position failed: {err}");
        return false;
    }
    if (x, y, width, height) != (state.x, state.y, state.width, state.height) {
        log::info!(
            "[window-state] restored geometry clamped to work area: saved x={} y={} w={} h={} -> applied x={} y={} w={} h={}",
            state.x,
            state.y,
            state.width,
            state.height,
            x,
            y,
            width,
            height
        );
    } else {
        log::info!(
            "[window-state] restored geometry x={} y={} w={} h={}",
            x,
            y,
            width,
            height
        );
    }
    true
}

/// Center the main window on the primary display (or its current monitor
/// if `current_monitor` resolves) when no saved state applied.
///
/// Also clamps the current outer size to fit inside the chosen monitor's
/// work area so the default 1000×800 declared in `tauri.conf.json` does
/// not exceed the user's actual screen on small or scaled displays
/// (issue #2282).
pub fn center_main<R: Runtime>(window: &WebviewWindow<R>) {
    let Some(monitor) = primary_or_current_work_area(window) else {
        let _ = window.center();
        return;
    };
    let Ok(size) = window.outer_size() else {
        let _ = window.center();
        return;
    };

    // Resolve the new size first; if the default exceeds work area we
    // shrink before centering so the centered position is computed
    // against the actually-applied size, not the oversized default.
    let (clamped_w, clamped_h) = clamp_size(size.width, size.height, &monitor);
    if (clamped_w, clamped_h) != (size.width, size.height) {
        log::info!(
            "[window-state] default size {}x{} exceeds work area {}x{}; shrinking to {}x{}",
            size.width,
            size.height,
            monitor.width,
            monitor.height,
            clamped_w,
            clamped_h,
        );
        if let Err(err) = window.set_size(PhysicalSize::new(clamped_w, clamped_h)) {
            log::warn!("[window-state] set_size during center failed: {err}");
        }
    }

    // Pathological-tiny-monitor case: when the work area is smaller
    // than `MIN_WINDOW_*`, `clamp_size` keeps the size at the minimum
    // floor, so `clamped_w/h` can still exceed `monitor.width/height`
    // and the naive center math would push the origin negative
    // (title bar off the left/top edge). Run the centered origin
    // through `clamp_to_work_area` so the title bar stays anchored at
    // the work-area top-left in that case — same fallback `restore_main`
    // already gets for free.
    let centered_x = monitor.x + (monitor.width as i32 - clamped_w as i32) / 2;
    let centered_y = monitor.y + (monitor.height as i32 - clamped_h as i32) / 2;
    let (x, y, _, _) = clamp_to_work_area(centered_x, centered_y, clamped_w, clamped_h, monitor);
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("[window-state] set_position during center failed: {err}");
    }
}

fn collect_work_areas<R: Runtime>(window: &WebviewWindow<R>) -> Vec<WorkArea> {
    let Ok(monitors) = window.available_monitors() else {
        return Vec::new();
    };
    monitors
        .iter()
        .map(|m| {
            let wa = m.work_area();
            WorkArea {
                x: wa.position.x,
                y: wa.position.y,
                width: wa.size.width,
                height: wa.size.height,
            }
        })
        .collect()
}

fn primary_or_current_work_area<R: Runtime>(window: &WebviewWindow<R>) -> Option<WorkArea> {
    let monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten())?;
    let wa = monitor.work_area();
    Some(WorkArea {
        x: wa.position.x,
        y: wa.position.y,
        width: wa.size.width,
        height: wa.size.height,
    })
}

/// Return the work area whose intersection with the saved window rect
/// has the **largest area** while still meeting `MIN_VISIBLE_OVERLAP_PX`
/// on each axis. When the user undocks a display the saved coordinates
/// land in nowhere-land and this returns `None` so the caller can fall
/// back to a fresh centered default.
///
/// Picking by largest overlap (rather than the first qualifying monitor)
/// keeps multi-monitor restores deterministic: a window straddling two
/// screens lands on the one that actually contained most of it before
/// the restart, independent of `available_monitors()` ordering.
fn pick_monitor_for_window(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    work_areas: &[WorkArea],
) -> Option<WorkArea> {
    let win_right = x.saturating_add(width as i32);
    let win_bottom = y.saturating_add(height as i32);
    work_areas
        .iter()
        .copied()
        .filter_map(|wa| {
            let mon_right = wa.x.saturating_add(wa.width as i32);
            let mon_bottom = wa.y.saturating_add(wa.height as i32);
            let overlap_w = (win_right.min(mon_right) - x.max(wa.x)).max(0);
            let overlap_h = (win_bottom.min(mon_bottom) - y.max(wa.y)).max(0);
            if overlap_w >= MIN_VISIBLE_OVERLAP_PX && overlap_h >= MIN_VISIBLE_OVERLAP_PX {
                // i64 widening keeps the product safe against
                // pathological monitor sizes near `i32::MAX`.
                Some((i64::from(overlap_w) * i64::from(overlap_h), wa))
            } else {
                None
            }
        })
        .max_by_key(|(area, _)| *area)
        .map(|(_, wa)| wa)
}

/// Clamp width/height into the work area while preserving the
/// `MIN_WINDOW_*` floors. Pure helper extracted from
/// [`clamp_to_work_area`] so `center_main` can reuse it when the window
/// already has the position it wants and only needs the size capped.
fn clamp_size(width: u32, height: u32, work_area: &WorkArea) -> (u32, u32) {
    let max_w = work_area.width.max(MIN_WINDOW_WIDTH);
    let max_h = work_area.height.max(MIN_WINDOW_HEIGHT);
    let w = width.clamp(MIN_WINDOW_WIDTH, max_w);
    let h = height.clamp(MIN_WINDOW_HEIGHT, max_h);
    (w, h)
}

/// Clamp `(x, y, width, height)` into `work_area` so the entire window
/// frame lies within the work area. Size shrinks first; position then
/// shifts to keep the right/bottom edges inside the work area.
fn clamp_to_work_area(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    work_area: WorkArea,
) -> (i32, i32, u32, u32) {
    let (w, h) = clamp_size(width, height, &work_area);

    let wa_right = work_area.x.saturating_add(work_area.width as i32);
    let wa_bottom = work_area.y.saturating_add(work_area.height as i32);
    let max_x = wa_right.saturating_sub(w as i32);
    let max_y = wa_bottom.saturating_sub(h as i32);

    let clamped_x = x.clamp(work_area.x, max_x.max(work_area.x));
    let clamped_y = y.clamp(work_area.y, max_y.max(work_area.y));

    (clamped_x, clamped_y, w, h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wa(x: i32, y: i32, width: u32, height: u32) -> WorkArea {
        WorkArea {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn clamp_leaves_in_bounds_geometry_alone() {
        // 1280×800 work area, 1000×800 window centered-ish: width fits,
        // height fits exactly — nothing should change.
        let work_area = wa(0, 0, 1280, 800);
        let (x, y, w, h) = clamp_to_work_area(100, 0, 1000, 800, work_area);
        assert_eq!((x, y, w, h), (100, 0, 1000, 800));
    }

    #[test]
    fn clamp_shrinks_window_taller_than_work_area() {
        // Repro for #2282: default 1000×800 on a 1280×720 work area
        // (e.g. macOS 13" Air with menu bar+dock visible). Height
        // shrinks to the work area height so bottom nav stays visible.
        let work_area = wa(0, 0, 1280, 720);
        let (x, y, w, h) = clamp_to_work_area(0, 0, 1000, 800, work_area);
        assert_eq!(w, 1000);
        assert_eq!(h, 720);
        assert_eq!((x, y), (0, 0));
    }

    #[test]
    fn clamp_shrinks_window_wider_than_work_area() {
        let work_area = wa(0, 0, 800, 600);
        let (_, _, w, h) = clamp_to_work_area(0, 0, 1600, 1200, work_area);
        assert_eq!(w, 800);
        assert_eq!(h, 600);
    }

    #[test]
    fn clamp_respects_minimum_size_on_tiny_work_area() {
        // Pathological tiny work area: don't shrink below the usability
        // floor. (User can still scroll/resize; better than a 0×0 sliver.)
        let work_area = wa(0, 0, 200, 150);
        let (_, _, w, h) = clamp_to_work_area(0, 0, 1000, 800, work_area);
        assert_eq!(w, MIN_WINDOW_WIDTH);
        assert_eq!(h, MIN_WINDOW_HEIGHT);
    }

    #[test]
    fn clamp_pushes_window_back_inside_when_off_right_or_bottom() {
        // Saved at (1200, 700) sized 1000×800 — bottom-right is far
        // outside the 1280×800 work area. Position should shift left
        // and up so the *whole frame* fits inside the work area.
        let work_area = wa(0, 0, 1280, 800);
        let (x, y, w, h) = clamp_to_work_area(1200, 700, 1000, 800, work_area);
        assert_eq!(w, 1000);
        assert_eq!(h, 800);
        assert_eq!(x, 1280 - 1000);
        assert_eq!(y, 0);
    }

    #[test]
    fn clamp_handles_negative_position_on_offset_monitor() {
        // Secondary monitor positioned to the left of the primary —
        // origin is at (-1920, 0). A saved window slightly left of that
        // monitor's left edge should be pulled inward.
        let work_area = wa(-1920, 0, 1920, 1080);
        let (x, y, _, _) = clamp_to_work_area(-2000, 100, 1000, 800, work_area);
        assert_eq!(x, -1920);
        assert_eq!(y, 100);
    }

    #[test]
    fn clamp_size_only_caps_to_work_area() {
        let work_area = wa(0, 0, 1024, 600);
        let (w, h) = clamp_size(1600, 1200, &work_area);
        assert_eq!((w, h), (1024, 600));
    }

    #[test]
    fn clamp_size_below_minimum_floor_returns_minimum() {
        let work_area = wa(0, 0, 200, 150);
        let (w, h) = clamp_size(100, 50, &work_area);
        assert_eq!((w, h), (MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT));
    }

    #[test]
    fn pick_monitor_finds_overlapping_monitor() {
        let monitors = vec![wa(0, 0, 1920, 1080), wa(1920, 0, 1280, 800)];
        // Window sits on the secondary monitor (right of primary).
        let m = pick_monitor_for_window(2000, 100, 1000, 700, &monitors).unwrap();
        assert_eq!((m.x, m.width), (1920, 1280));
    }

    #[test]
    fn pick_monitor_returns_none_when_window_off_every_screen() {
        // Saved on a now-disconnected display (large positive offset).
        let monitors = vec![wa(0, 0, 1920, 1080)];
        let m = pick_monitor_for_window(5000, 5000, 1000, 800, &monitors);
        assert!(
            m.is_none(),
            "off-screen window should not match any monitor"
        );
    }

    #[test]
    fn pick_monitor_requires_minimum_overlap() {
        // Window only intersects the monitor by a 50px sliver — below
        // the 100px threshold, so we treat it as off-screen.
        let monitors = vec![wa(0, 0, 1920, 1080)];
        let m = pick_monitor_for_window(-950, 100, 1000, 700, &monitors);
        assert!(m.is_none(), "sub-threshold overlap should not match");
    }

    #[test]
    fn pick_monitor_handles_empty_list() {
        let m = pick_monitor_for_window(0, 0, 1000, 800, &[]);
        assert!(m.is_none());
    }

    #[test]
    fn pick_monitor_prefers_largest_overlap_for_straddling_window() {
        // Window at (1820, 0) with 1000×700 straddles two horizontally
        // adjacent monitors: 100×700 = 70 000 px² on the primary,
        // 900×700 = 630 000 px² on the secondary. Must pick the
        // secondary regardless of `available_monitors()` ordering.
        let primary = wa(0, 0, 1920, 1080);
        let secondary = wa(1920, 0, 1280, 800);
        // Primary first.
        let m = pick_monitor_for_window(1820, 0, 1000, 700, &[primary, secondary]).unwrap();
        assert_eq!((m.x, m.width), (1920, 1280));
        // Secondary first — same answer.
        let m = pick_monitor_for_window(1820, 0, 1000, 700, &[secondary, primary]).unwrap();
        assert_eq!((m.x, m.width), (1920, 1280));
    }

    #[test]
    fn center_origin_after_min_floor_stays_in_work_area() {
        // Repro for the `center_main` edge case: a pathological work
        // area smaller than `MIN_WINDOW_*` forces the size to stay at
        // the minimum floor (480×360), which is larger than the work
        // area itself (e.g. 200×150). The naive centered origin would
        // be `(200 - 480)/2 = -140` — title bar off-screen. Running
        // the centered origin through `clamp_to_work_area` must pin it
        // back to the work-area top-left so the title bar is at least
        // reachable.
        let work_area = wa(0, 0, 200, 150);
        let clamped_w = MIN_WINDOW_WIDTH;
        let clamped_h = MIN_WINDOW_HEIGHT;
        let centered_x = work_area.x + (work_area.width as i32 - clamped_w as i32) / 2;
        let centered_y = work_area.y + (work_area.height as i32 - clamped_h as i32) / 2;
        assert!(
            centered_x < work_area.x,
            "precondition: naive center should land off-screen"
        );
        let (x, y, w, h) =
            clamp_to_work_area(centered_x, centered_y, clamped_w, clamped_h, work_area);
        assert_eq!((x, y), (work_area.x, work_area.y));
        assert_eq!((w, h), (clamped_w, clamped_h));
    }
}
