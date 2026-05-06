//! Tauri shell side of file-based logging.
//!
//! Resolves the OpenHuman data directory the same way the core does
//! (`~/.openhuman` or `OPENHUMAN_WORKSPACE` override) and hands it to
//! [`openhuman_core::core::logging::init_for_embedded`], which installs a
//! daily-rotated file appender so packaged GUI builds — where stderr is
//! invisible — still produce a log users can share for support.
//!
//! Both the shell's `log::*` calls (via the `tracing_log::LogTracer` bridge)
//! and the embedded core's `tracing::*` events funnel into the same file.

use std::path::PathBuf;

use openhuman_core::core::logging::{self, log_directory};

/// Initialize logging for the Tauri shell + embedded core. Idempotent and
/// safe to call from any startup position; the underlying `Once` guard means
/// the first caller's data dir wins.
///
/// Verbosity defaults to `info` (or `debug` when `OPENHUMAN_VERBOSE=1`); the
/// `RUST_LOG` env var continues to override both.
pub fn init() {
    let data_dir = resolve_data_dir();
    let verbose = std::env::var("OPENHUMAN_VERBOSE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    logging::init_for_embedded(&data_dir, verbose);
}

/// Resolve the directory used to host `<data_dir>/logs/`. Mirrors the core's
/// own resolution so log files sit next to `active_user.toml`, the per-user
/// `users/` tree, and the CEF caches a support engineer would also need.
fn resolve_data_dir() -> PathBuf {
    if let Ok(workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
        if !workspace.is_empty() {
            return PathBuf::from(workspace);
        }
    }
    openhuman_core::openhuman::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| PathBuf::from(".openhuman"))
}

/// Tauri command — return the absolute path to the active log directory, or
/// `None` if logging hasn't been initialized in embedded mode (shouldn't
/// happen at runtime; guard for tests).
#[tauri::command]
pub fn logs_folder_path() -> Option<String> {
    log_directory().map(|p| p.display().to_string())
}

/// Tauri command — open the platform file manager at the log directory so a
/// user can grab today's log file and send it to support.
#[tauri::command]
pub fn reveal_logs_folder() -> Result<(), String> {
    let dir = log_directory().ok_or_else(|| "log directory not initialized".to_string())?;

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(dir).spawn();

    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg(dir).spawn();

    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(dir).spawn();

    result
        .map(|_| ())
        .map_err(|e| format!("failed to open log directory {}: {e}", dir.display()))
}
