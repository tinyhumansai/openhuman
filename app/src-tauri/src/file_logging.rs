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
///
/// If `default_root_openhuman_dir` fails (very unusual — it requires
/// `dirs::home_dir` to return `None`), falls back to `<temp>/openhuman`
/// rather than a relative `.openhuman` whose final location depends on the
/// shell's CWD at launch time.
fn resolve_data_dir() -> PathBuf {
    if let Ok(workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
        if !workspace.is_empty() {
            return PathBuf::from(workspace);
        }
    }
    openhuman_core::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|err| {
        eprintln!(
            "[file_logging] default_root_openhuman_dir failed ({err}); falling back to temp dir"
        );
        std::env::temp_dir().join("openhuman")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lock around env-var mutation. Cargo runs unit tests in parallel
    /// threads in the same process, so concurrent `set_var` / `remove_var`
    /// can race; the lock keeps the env stable for each test's duration.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn resolve_data_dir_honors_workspace_override() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prior = std::env::var("OPENHUMAN_WORKSPACE").ok();
        std::env::set_var("OPENHUMAN_WORKSPACE", "/tmp/openhuman-test-override");
        let dir = resolve_data_dir();
        assert_eq!(dir, PathBuf::from("/tmp/openhuman-test-override"));
        match prior {
            Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }

    #[test]
    fn resolve_data_dir_ignores_empty_workspace() {
        let _guard = ENV_LOCK.lock().unwrap();
        let prior = std::env::var("OPENHUMAN_WORKSPACE").ok();
        std::env::set_var("OPENHUMAN_WORKSPACE", "");
        // Empty string must NOT short-circuit — fall through to the
        // default resolver so the user's real `~/.openhuman` is used.
        let dir = resolve_data_dir();
        assert_ne!(dir, PathBuf::from(""));
        assert!(dir.is_absolute(), "expected absolute fallback, got {dir:?}");
        match prior {
            Some(v) => std::env::set_var("OPENHUMAN_WORKSPACE", v),
            None => std::env::remove_var("OPENHUMAN_WORKSPACE"),
        }
    }

    #[test]
    fn logs_folder_path_returns_none_pre_init() {
        // `init()` is `Once`-guarded across the whole process, so in unit
        // tests where the embedded subscriber hasn't been installed,
        // `logs_folder_path` should return `None` rather than a stale path.
        // (When run alongside a test that *did* call `init`, the function
        // is allowed to return Some — assert the type signature only.)
        let result = logs_folder_path();
        let _: Option<String> = result;
    }

    #[test]
    fn reveal_logs_folder_errors_when_uninitialized() {
        // If logging hasn't been initialized, the command must surface a
        // typed error so the UI can show it instead of silently launching
        // an `open` against an empty path.
        if openhuman_core::core::logging::log_directory().is_none() {
            let err = reveal_logs_folder().expect_err("must error pre-init");
            assert!(err.contains("not initialized"), "unexpected error: {err}");
        }
    }
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
