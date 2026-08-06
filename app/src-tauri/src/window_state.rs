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
//!
//! ## Mixed-DPI multi-monitor (#5041)
//!
//! Every coordinate in this module is a **physical** pixel:
//! `Monitor::work_area()`, `outer_position()` and `outer_size()` all
//! speak physical, so there is no logical/physical conversion to get
//! wrong. The DPI hazard is elsewhere — Windows preserves a window's
//! *logical* size across a monitor change, so moving a window from a
//! 100 % display to a 150 % one makes the OS multiply its physical size
//! by 1.5 on arrival. A size that was correctly clamped to the
//! destination monitor before the move therefore arrives 1.5x too large.
//!
//! Two mitigations, because the OS rescale is asynchronous:
//!
//! 1. [`center_main`] targets the monitor the window is *already* on
//!    (avoiding the cross-DPI move entirely), and when a move is
//!    unavoidable it positions **before** sizing so the rescale lands
//!    first and the applied size is measured against the destination.
//! 2. [`install_dpi_guard`] re-clamps on `ScaleFactorChanged`, which is
//!    delivered through the message loop *after* `setup()` has returned
//!    — the case no amount of startup clamping can catch.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{PhysicalPosition, PhysicalSize, Runtime, WebviewWindow, WindowEvent};

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

/// A monitor's usable work area in physical pixels, plus its DPI scale
/// factor. Plain-data struct so the geometry math in
/// [`clamp_to_work_area`] / [`pick_monitor_for_window`] /
/// [`dpi_adjusted_size`] can be unit-tested without a live Tauri runtime.
///
/// `scale` is only meaningful for cross-monitor moves on Windows — see
/// [`dpi_adjusted_size`]. All four geometry fields are physical pixels,
/// matching `Monitor::work_area()`, `outer_position()` and
/// `outer_size()`, so no logical/physical conversion happens anywhere in
/// this module.
#[derive(Debug, Clone, Copy)]
struct WorkArea {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale: f64,
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

    // Position before size (#5041). The saved geometry may belong to a
    // monitor with a different scale factor than the one the window was
    // just created on; moving first lets the OS apply its DPI rescale,
    // so the size we then set is the size that sticks. The reverse order
    // lets Windows multiply our just-applied size by the DPI ratio on
    // arrival. `install_dpi_guard` still backstops the async case.
    if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
        log::warn!("[window-state] set_position failed: {err}");
        return false;
    }
    if let Err(err) = window.set_size(PhysicalSize::new(width, height)) {
        log::warn!("[window-state] set_size failed: {err}");
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
    let Ok(size) = window.outer_size() else {
        let _ = window.center();
        return;
    };
    let work_areas = collect_work_areas(window);
    let primary = primary_or_current_work_area(window);
    // Where the window currently sits drives monitor selection. With no
    // readable position there is nothing to overlap-test, so go straight
    // to the primary.
    let target = match window.outer_position() {
        Ok(pos) => target_work_area_for_new_window(
            pos.x,
            pos.y,
            size.width,
            size.height,
            &work_areas,
            primary,
        ),
        Err(err) => {
            log::warn!("[window-state] outer_position unavailable ({err}); targeting primary");
            primary.or_else(|| work_areas.first().copied())
        }
    };
    let Some(monitor) = target else {
        let _ = window.center();
        return;
    };

    // Scale factor the window is living at *right now*. On a cross-DPI
    // move the OS will rescale by `monitor.scale / current_scale`, so we
    // need both halves of the ratio.
    let current_scale = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| m.scale_factor())
        .or_else(|| window.scale_factor().ok())
        .unwrap_or(monitor.scale);

    // Move onto the target monitor BEFORE committing a size (#5041).
    //
    // Ordering is load-bearing: `set_size` then `set_position` lets
    // Windows rescale the just-applied size on arrival, which is exactly
    // how a correctly-clamped window ends up 1.5x too wide. Positioning
    // first lets the DPI change land, so the size we then apply is
    // measured against the monitor the window is actually on.
    if (monitor.scale - current_scale).abs() > f64::EPSILON {
        let (predicted_w, predicted_h) =
            dpi_adjusted_size(size.width, size.height, current_scale, monitor.scale);
        log::info!(
            "[window-state] cross-DPI placement: scale {} -> {}; size {}x{} would become {}x{} on arrival; positioning before sizing",
            current_scale,
            monitor.scale,
            size.width,
            size.height,
            predicted_w,
            predicted_h,
        );
        if let Err(err) = window.set_position(PhysicalPosition::new(monitor.x, monitor.y)) {
            log::warn!("[window-state] pre-size set_position failed: {err}");
        }
    }

    // Re-read the size: on the cross-DPI path above the OS may already
    // have rescaled us, so `size` is stale and clamping it would apply
    // the wrong number.
    let settled = window.outer_size().unwrap_or(size);
    if (settled.width, settled.height) != (size.width, size.height) {
        log::info!(
            "[window-state] OS rescaled window during placement: {}x{} -> {}x{}",
            size.width,
            size.height,
            settled.width,
            settled.height,
        );
    }

    // Resolve the new size first; if the default exceeds work area we
    // shrink before centering so the centered position is computed
    // against the actually-applied size, not the oversized default.
    let (clamped_w, clamped_h) = clamp_size(settled.width, settled.height, &monitor);
    if (clamped_w, clamped_h) != (settled.width, settled.height) {
        log::info!(
            "[window-state] size {}x{} exceeds work area {}x{}; shrinking to {}x{}",
            settled.width,
            settled.height,
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

/// Re-clamp the window into its current monitor's work area after the OS
/// has changed its scale factor.
///
/// `center_main` and `restore_main` both run inside Tauri's `setup()`
/// hook, but `WM_DPICHANGED` is delivered through the message loop
/// *after* `setup()` returns. Any size the OS imposes on arrival at a
/// different-DPI monitor therefore lands after our clamping has already
/// finished — the "clamping runs too late" half of #5041. This handler
/// is the durable fix: whenever the scale factor changes, re-fit the
/// window to whichever monitor it is now on.
///
/// It also covers the case the startup path cannot: the user dragging
/// the window from a 100 % monitor onto a 150 % one at any point during
/// the session, where the same OS rescale applies.
fn reclamp_after_scale_change<R: Runtime>(window: &WebviewWindow<R>, new_scale: f64) {
    // Maximized and fullscreen windows intentionally carry geometry that can
    // exceed the work area, and the OS owns it in those states. Clamping here
    // would issue `set_size`/`set_position` that unmaximizes the window, or
    // overwrite the bounds the OS restores on leaving fullscreen. Both states
    // are reachable from the app's own `toggleMaximize()` and the native
    // fullscreen menu, so this is a normal path, not an edge case.
    //
    // A failed query is treated as "not in that state": the pre-#5041 behaviour
    // was to always clamp, so falling back to clamping keeps the fix working
    // rather than silently disabling it on a backend that cannot answer.
    if window.is_maximized().unwrap_or(false) || window.is_fullscreen().unwrap_or(false) {
        log::debug!(
            "[window-state] scale change to {new_scale}; window is maximized/fullscreen — leaving geometry to the OS"
        );
        return;
    }

    let Ok(Some(monitor)) = window.current_monitor() else {
        log::warn!("[window-state] scale change but current_monitor unavailable; skip re-clamp");
        return;
    };
    let work_area = work_area_of(&monitor);
    let Ok(pos) = window.outer_position() else {
        log::warn!("[window-state] scale change but outer_position unavailable; skip re-clamp");
        return;
    };
    let Ok(size) = window.outer_size() else {
        log::warn!("[window-state] scale change but outer_size unavailable; skip re-clamp");
        return;
    };

    let (x, y, w, h) = clamp_to_work_area(pos.x, pos.y, size.width, size.height, work_area);
    if (x, y, w, h) == (pos.x, pos.y, size.width, size.height) {
        log::debug!(
            "[window-state] scale change to {new_scale}; geometry {}x{} at ({},{}) already fits work area",
            size.width,
            size.height,
            pos.x,
            pos.y
        );
        return;
    }

    log::info!(
        "[window-state] scale change to {new_scale}; re-clamping {}x{} at ({},{}) -> {}x{} at ({},{}) for work area ({},{} {}x{})",
        size.width,
        size.height,
        pos.x,
        pos.y,
        w,
        h,
        x,
        y,
        work_area.x,
        work_area.y,
        work_area.width,
        work_area.height,
    );
    // Size before position: shrinking first means the subsequent move
    // has a frame that already fits, so it cannot be pushed back out.
    // `clamp_to_work_area` only ever shrinks and shifts within this one
    // monitor, so this cannot bounce the window onto another display and
    // re-trigger the handler.
    if (w, h) != (size.width, size.height) {
        if let Err(err) = window.set_size(PhysicalSize::new(w, h)) {
            log::warn!("[window-state] set_size during scale-change re-clamp failed: {err}");
        }
    }
    if (x, y) != (pos.x, pos.y) {
        if let Err(err) = window.set_position(PhysicalPosition::new(x, y)) {
            log::warn!("[window-state] set_position during scale-change re-clamp failed: {err}");
        }
    }
}

/// Subscribe the main window to scale-factor changes so it is re-fitted
/// whenever the OS moves it across a DPI boundary (#5041).
///
/// Call once, from `setup()`, alongside `restore_main` / `center_main`.
pub fn install_dpi_guard<R: Runtime>(window: &WebviewWindow<R>) {
    let guarded = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::ScaleFactorChanged { scale_factor, .. } = event {
            reclamp_after_scale_change(&guarded, *scale_factor);
        }
    });
    log::info!("[window-state] DPI guard installed on main window");
}

/// Project a live `Monitor` into the plain-data [`WorkArea`] the pure
/// geometry helpers operate on. Single conversion point so the
/// physical-pixel contract is stated once.
fn work_area_of(monitor: &tauri::Monitor) -> WorkArea {
    let wa = monitor.work_area();
    WorkArea {
        x: wa.position.x,
        y: wa.position.y,
        width: wa.size.width,
        height: wa.size.height,
        scale: monitor.scale_factor(),
    }
}

fn collect_work_areas<R: Runtime>(window: &WebviewWindow<R>) -> Vec<WorkArea> {
    let Ok(monitors) = window.available_monitors() else {
        return Vec::new();
    };
    monitors.iter().map(work_area_of).collect()
}

fn primary_or_current_work_area<R: Runtime>(window: &WebviewWindow<R>) -> Option<WorkArea> {
    let monitor = window
        .primary_monitor()
        .ok()
        .flatten()
        .or_else(|| window.current_monitor().ok().flatten())?;
    Some(work_area_of(&monitor))
}

/// Emit the full monitor layout once at startup.
///
/// Mixed-DPI multi-monitor bugs (#5041) are effectively undebuggable from
/// a user report without knowing each monitor's origin, work area and
/// scale factor — the reporter's own numbers came from a third-party
/// tool and could not be reconciled afterwards. Logged at info so it
/// lands in the shipped log file.
pub fn log_monitor_layout<R: Runtime>(window: &WebviewWindow<R>) {
    let Ok(monitors) = window.available_monitors() else {
        log::warn!("[window-state] available_monitors unavailable; cannot log layout");
        return;
    };
    let primary_name = window
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|m| m.name().cloned());
    log::info!(
        "[window-state] monitor layout: {} attached, primary={:?}",
        monitors.len(),
        primary_name
    );
    for (idx, m) in monitors.iter().enumerate() {
        let wa = m.work_area();
        let size = m.size();
        log::info!(
            "[window-state]   monitor[{}] name={:?} scale={} full={}x{} work_area=({},{} {}x{})",
            idx,
            m.name(),
            m.scale_factor(),
            size.width,
            size.height,
            wa.position.x,
            wa.position.y,
            wa.size.width,
            wa.size.height,
        );
    }
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

/// Pick the monitor a freshly-created window should be sized and centered
/// against.
///
/// Preference order:
/// 1. The monitor the window is **actually on** (largest work-area
///    overlap). Staying put avoids a cross-monitor move entirely, which
///    on Windows is what triggers the DPI rescale described in
///    [`dpi_adjusted_size`].
/// 2. The primary monitor, when the window overlaps nothing (Windows can
///    place a not-yet-shown window at `CW_USEDEFAULT`, off every work
///    area).
/// 3. Any attached monitor, so we never return `None` while monitors exist.
///
/// The previous behaviour — always prefer the primary — is what made the
/// mixed-DPI case (#5041) reachable: a window created on a 100 % monitor
/// was clamped against the 150 % primary's work area and then moved
/// there, and Windows rescaled the just-applied size by 1.5 on arrival.
fn target_work_area_for_new_window(
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    work_areas: &[WorkArea],
    primary: Option<WorkArea>,
) -> Option<WorkArea> {
    pick_monitor_for_window(x, y, width, height, work_areas)
        .or(primary)
        .or_else(|| work_areas.first().copied())
}

/// Predict the physical size an OS will impose on a window that moves
/// between monitors with different DPI scale factors.
///
/// This is the crux of #5041. Windows preserves a window's **logical**
/// size across a monitor change: on `WM_DPICHANGED` it multiplies the
/// physical size by `to_scale / from_scale`. So setting a physical size
/// that fits monitor B, and *then* moving the window from monitor A to
/// monitor B, does not leave the window at that size — it arrives
/// `to_scale / from_scale` times larger.
///
/// Concretely, for the reporter's layout (secondary 1920×1080 @ 100 %,
/// primary 2560×1600 @ 150 %): a window correctly clamped to the
/// primary's 2560-px width while still sitting on the secondary arrives
/// at `2560 × 1.5 = 3840` physical pixels — wider than the 2560-px
/// monitor it was just fitted to, overflowing off the right edge.
///
/// Returns the input unchanged when either scale is not a usable
/// positive, finite number, so a runtime reporting `0.0` or `NaN` can
/// never zero out or panic the window size.
fn dpi_adjusted_size(width: u32, height: u32, from_scale: f64, to_scale: f64) -> (u32, u32) {
    let usable = |s: f64| s.is_finite() && s > 0.0;
    if !usable(from_scale) || !usable(to_scale) {
        return (width, height);
    }
    let ratio = to_scale / from_scale;
    if !ratio.is_finite() || ratio <= 0.0 {
        return (width, height);
    }
    let scaled = |v: u32| {
        // Round rather than truncate. Truncation always loses, so a
        // window shuttled between two monitors shrinks by up to a pixel
        // per move and the error accumulates without bound. Rounding
        // keeps it bounded at ±1 px total, however many moves happen.
        //
        // It is NOT an exact inverse in general. When the first hop is a
        // *downscale* the intermediate size has genuinely lost
        // information, and scaling back up cannot recover it: 1281 px at
        // 2.0→1.25 gives 801, and 801 back at 1.25→2.0 gives 1282. The
        // round trip is exact when the first hop scales up (the common
        // case: a window sized on a low-DPI monitor moving to a high-DPI
        // one and back). `dpi_adjusted_size_round_trips_without_drift`
        // pins both halves of that contract.
        let out = (f64::from(v) * ratio).round();
        // Saturate instead of wrapping — an absurd ratio must not
        // produce a tiny window via u32 overflow.
        if out >= f64::from(u32::MAX) {
            u32::MAX
        } else if out <= 0.0 {
            1
        } else {
            out as u32
        }
    };
    (scaled(width), scaled(height))
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

    #[test]
    fn window_state_toml_roundtrips() {
        // `save_main` serializes this record to `window_state.toml` and
        // `restore_main` parses it back on the next launch. The whole
        // save-on-quit → restore-on-launch feature (#4810) hinges on this
        // record surviving the round trip byte-for-byte, including a
        // negative x from a monitor left of the primary. Guards the
        // on-disk contract against an accidental serde/rename change.
        let state = WindowState {
            x: -1920,
            y: 100,
            width: 1280,
            height: 800,
        };
        let raw = toml::to_string_pretty(&state).expect("serialize");
        let parsed: WindowState = toml::from_str(&raw).expect("parse");
        assert_eq!(
            (parsed.x, parsed.y, parsed.width, parsed.height),
            (state.x, state.y, state.width, state.height)
        );
    }

    fn wa(x: i32, y: i32, width: u32, height: u32) -> WorkArea {
        wa_dpi(x, y, width, height, 1.0)
    }

    /// Same as [`wa`] but with an explicit DPI scale factor, for the
    /// mixed-DPI cases in #5041.
    fn wa_dpi(x: i32, y: i32, width: u32, height: u32, scale: f64) -> WorkArea {
        WorkArea {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    /// The reporter's layout from #5041: a 2560x1600 @ 150 % primary at
    /// the virtual-desktop origin, and a 1920x1080 @ 100 % secondary
    /// placed to its **left** (so the secondary's origin is negative).
    /// Work areas subtract a plausible taskbar.
    fn reporter_layout() -> (WorkArea, WorkArea) {
        let primary = wa_dpi(0, 0, 2560, 1520, 1.5);
        let secondary = wa_dpi(-1920, 0, 1920, 1032, 1.0);
        (primary, secondary)
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

    // ── Mixed-DPI multi-monitor (#5041) ──────────────────────────────

    #[test]
    fn dpi_adjusted_size_grows_when_moving_to_a_higher_dpi_monitor() {
        // The #5041 arithmetic: a window fitted to the 150 % primary's
        // 2560 px work-area width while still sitting on the 100 %
        // secondary arrives at 3840 px — wider than the very monitor it
        // was just fitted to. This is the reported ~3875 px window.
        let (grown_w, grown_h) = dpi_adjusted_size(2560, 1520, 1.0, 1.5);
        assert_eq!(grown_w, 3840);
        assert_eq!(grown_h, 2280);
    }

    #[test]
    fn dpi_adjusted_size_shrinks_when_moving_to_a_lower_dpi_monitor() {
        let (w, h) = dpi_adjusted_size(3840, 2280, 1.5, 1.0);
        assert_eq!((w, h), (2560, 1520));
    }

    #[test]
    fn dpi_adjusted_size_is_identity_at_equal_scale() {
        assert_eq!(dpi_adjusted_size(1280, 900, 1.5, 1.5), (1280, 900));
        assert_eq!(dpi_adjusted_size(1280, 900, 1.0, 1.0), (1280, 900));
    }

    #[test]
    fn dpi_adjusted_size_round_trips_without_drift() {
        // A→B→A must land back on the original size; truncating instead
        // of rounding would lose a pixel on every move.
        //
        // Covers the whole standard DPI ladder rather than just 1.0↔1.5:
        // a single pair does not establish the property (review, #5041).
        // Odd dimensions are deliberate — even ones round-trip trivially.
        const SIZES: [(u32, u32); 3] = [(1281, 901), (1920, 1080), (2560, 1600)];

        // Upscale first: exact. This is the common case — a window sized
        // on a low-DPI monitor moves to a high-DPI one and back.
        for (from, to) in [
            (1.0, 1.25),
            (1.0, 1.5),
            (1.0, 1.75),
            (1.0, 2.0),
            (1.25, 1.5),
            (1.5, 2.0),
        ] {
            for (w, h) in SIZES {
                let (up_w, up_h) = dpi_adjusted_size(w, h, from, to);
                let (back_w, back_h) = dpi_adjusted_size(up_w, up_h, to, from);
                assert_eq!(
                    (back_w, back_h),
                    (w, h),
                    "upscale-first round trip {w}x{h} at {from}->{to}->{from} drifted to {back_w}x{back_h}"
                );
            }
        }

        // Downscale first: bounded, not exact. The intermediate size has
        // genuinely lost information (1281 at 2.0->1.25 is 801, and 801
        // back up is 1282), so the contract is that the error stays
        // within 1 px and never accumulates — which is precisely what
        // truncation fails to do.
        for (from, to) in [(2.0, 1.25), (1.75, 1.0), (2.0, 1.0), (1.5, 1.25)] {
            for (w, h) in SIZES {
                let (down_w, down_h) = dpi_adjusted_size(w, h, from, to);
                let (back_w, back_h) = dpi_adjusted_size(down_w, down_h, to, from);
                assert!(
                    back_w.abs_diff(w) <= 1 && back_h.abs_diff(h) <= 1,
                    "downscale-first round trip {w}x{h} at {from}->{to}->{from} drifted to {back_w}x{back_h}, beyond the 1px bound"
                );
            }
        }
    }

    #[test]
    fn dpi_adjusted_size_rejects_unusable_scale_factors() {
        // A runtime reporting 0.0 / NaN / infinity must not zero out or
        // panic the window size — return the input untouched instead.
        for (from, to) in [
            (0.0, 1.5),
            (1.5, 0.0),
            (f64::NAN, 1.5),
            (1.5, f64::NAN),
            (f64::INFINITY, 1.5),
            (1.5, f64::INFINITY),
            (-1.0, 1.5),
        ] {
            assert_eq!(
                dpi_adjusted_size(1280, 900, from, to),
                (1280, 900),
                "scale pair ({from}, {to}) must be treated as unusable"
            );
        }
    }

    #[test]
    fn target_prefers_the_monitor_the_window_is_already_on() {
        // Window created on the 100 % secondary (negative origin).
        // Selecting the primary here is what forces the cross-DPI move
        // that #5041 is about — we must stay put instead.
        let (primary, secondary) = reporter_layout();
        let target = target_work_area_for_new_window(
            -1800,
            100,
            1280,
            900,
            &[primary, secondary],
            Some(primary),
        )
        .expect("a monitor should be selected");
        assert_eq!(
            (target.x, target.width),
            (secondary.x, secondary.width),
            "window on the secondary must be sized against the secondary"
        );
        assert!(
            (target.scale - secondary.scale).abs() < f64::EPSILON,
            "and must carry the secondary's scale factor"
        );
    }

    #[test]
    fn target_falls_back_to_primary_when_window_overlaps_nothing() {
        // Windows can place a not-yet-shown window at CW_USEDEFAULT,
        // off every work area.
        let (primary, secondary) = reporter_layout();
        let target = target_work_area_for_new_window(
            50_000,
            50_000,
            1280,
            900,
            &[primary, secondary],
            Some(primary),
        )
        .expect("should fall back to primary");
        assert_eq!((target.x, target.width), (primary.x, primary.width));
    }

    #[test]
    fn target_falls_back_to_any_monitor_without_a_primary() {
        let (primary, secondary) = reporter_layout();
        let target =
            target_work_area_for_new_window(50_000, 50_000, 1280, 900, &[secondary, primary], None)
                .expect("should fall back to the first attached monitor");
        assert_eq!(target.x, secondary.x);
    }

    #[test]
    fn target_returns_none_without_monitors() {
        assert!(target_work_area_for_new_window(0, 0, 1280, 900, &[], None).is_none());
    }

    #[test]
    fn clamp_then_move_overflows_but_move_then_clamp_fits() {
        // End-to-end regression for #5041, expressed against the pure
        // helpers so it runs without a Tauri runtime.
        //
        // Setup: an oversized window (larger than either work area) is
        // created on the 100 % secondary and needs to end up on the
        // 150 % primary.
        let (primary, _secondary) = reporter_layout();
        let (created_w, created_h) = (3000, 1700);

        // OLD ordering — clamp to the destination, then move. The OS
        // rescales by 1.5 on arrival and the result overflows the very
        // monitor it was fitted to.
        let (clamped_w, clamped_h) = clamp_size(created_w, created_h, &primary);
        assert_eq!(
            (clamped_w, clamped_h),
            (2560, 1520),
            "clamp itself is correct"
        );
        let (arrived_w, arrived_h) = dpi_adjusted_size(clamped_w, clamped_h, 1.0, 1.5);
        assert!(
            arrived_w > primary.width && arrived_h > primary.height,
            "old ordering must reproduce the oversized window: {arrived_w}x{arrived_h} vs work area {}x{}",
            primary.width,
            primary.height
        );

        // NEW ordering — move first (absorbing the OS rescale), then
        // clamp against the destination. Result fits.
        let (moved_w, moved_h) = dpi_adjusted_size(created_w, created_h, 1.0, 1.5);
        let (final_w, final_h) = clamp_size(moved_w, moved_h, &primary);
        assert!(
            final_w <= primary.width && final_h <= primary.height,
            "new ordering must fit the work area: {final_w}x{final_h} vs {}x{}",
            primary.width,
            primary.height
        );
    }

    #[test]
    fn reclamped_geometry_never_leaves_the_work_area_after_a_scale_change() {
        // What `reclamp_after_scale_change` delegates to: whatever size
        // the OS imposed on arrival, the window must end up wholly
        // inside the destination work area.
        let (primary, _) = reporter_layout();
        let (os_w, os_h) = dpi_adjusted_size(2560, 1520, 1.0, 1.5);
        let (x, y, w, h) = clamp_to_work_area(0, 0, os_w, os_h, primary);
        assert!(w <= primary.width && h <= primary.height);
        assert!(x >= primary.x && y >= primary.y);
        assert!(x.saturating_add(w as i32) <= primary.x.saturating_add(primary.width as i32));
        assert!(y.saturating_add(h as i32) <= primary.y.saturating_add(primary.height as i32));
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
