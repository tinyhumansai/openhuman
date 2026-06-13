use crate::core_process;
#[cfg(target_os = "windows")]
use crate::reset_reboot_schedule;

/// Reset the user's local OpenHuman data and bounce the embedded core.
///
/// Replaces the prior two-step UI flow that called the core JSON-RPC
/// `openhuman.config_reset_local_data` (in-process removal) followed by
/// `restart_core_process`. The in-process removal failed on Windows with
/// `ERROR_SHARING_VIOLATION` (os error 32) because the running core held
/// open handles to SQLite databases, log files, the Sentry session store,
/// etc. inside the directory it was being asked to delete — see
/// OPENHUMAN-TAURI-AF.
///
/// New order:
///
/// 1. Query the core for the **paths** it would remove (`config_get_data_paths`)
///    while the core is still up — these are derived from the loaded config
///    and the active workspace marker, so the core is authoritative.
/// 2. Acquire the restart lock so a concurrent `restart_core_process` cannot
///    interleave with the remove.
/// 3. Shut down the embedded core. `CoreProcessHandle::shutdown` cancels
///    the cancellation token and awaits the tokio task, which drops the
///    SQLite pool, log writer, etc. — releasing every Windows file handle.
/// 4. Remove the three paths (current data dir, default data dir, active
///    workspace marker) from this process. Missing entries are non-fatal.
/// 5. Restart the embedded core via `ensure_running`.
///
/// Returns `Ok(())` only when the core is back up and the directories are
/// gone (or were already absent). Any step's `Err` short-circuits and
/// surfaces to the UI, which already renders the message as a toast.
#[tauri::command]
pub async fn reset_local_data(
    state: tauri::State<'_, core_process::CoreProcessHandle>,
) -> Result<(), String> {
    log::info!("[core] reset_local_data: command invoked from frontend");

    // ── 1. Ask the core for the paths it would remove ────────────────────
    //
    // The core is authoritative for path resolution (it owns config
    // loading, the workspace marker, and the staging-vs-prod default-dir
    // suffix). Resolve while the core is still up so we don't duplicate
    // that logic here.
    let paths = fetch_data_paths().await?;
    log::info!(
        "[core] reset_local_data: paths resolved current={} default={} marker={}",
        paths.current_openhuman_dir.display(),
        paths.default_openhuman_dir.display(),
        paths.active_workspace_marker_path.display()
    );

    // ── 2. Acquire the restart lock ─────────────────────────────────────
    //
    // Prevents a concurrent `restart_core_process` from re-spawning the
    // embedded server in the middle of the remove step.
    let _guard = state.inner().restart_lock().await;
    log::debug!("[core] reset_local_data: acquired restart lock");

    // ── 3. Shut down the embedded core ──────────────────────────────────
    //
    // Drops the tokio task, which drops the SQLite pool, log writer, and
    // every other RAII owner of a file handle inside the data directory.
    // On Windows this is the load-bearing step for OPENHUMAN-TAURI-AF.
    state.inner().shutdown().await;
    log::info!("[core] reset_local_data: embedded core stopped");

    // ── 3b. Release the host-process log file handle (issue #1615) ──────
    //
    // The daily-rotating log appender at `<data_dir>/logs/openhuman-*.log`
    // is owned by *this* Tauri host process, not by the embedded core
    // tokio task — so `shutdown()` above does not release it. On Windows
    // that lingering OS file handle causes `remove_dir_all(.openhuman)`
    // below to fail with `ERROR_SHARING_VIOLATION` (os error 32). Drop
    // the writer guard now so the background flushing thread exits and
    // the file handle is closed before the removal walks the tree.
    let log_guard_dropped = openhuman_core::core::logging::shutdown_file_guard();
    log::info!("[core] reset_local_data: shutdown_file_guard dropped guard = {log_guard_dropped}");

    // ── 4. Remove the paths ─────────────────────────────────────────────
    //
    // Missing entries are non-fatal: the user may already have manually
    // cleared the dir, or the marker may not exist for fresh installs.
    //
    // Capture the first delete error (if any) instead of propagating with
    // `?` — we must still restart the embedded core in step 5 so the app
    // doesn't end up with the sidecar dead. The original delete error is
    // surfaced after the restart attempt.
    let delete_result: Result<(), String> = async {
        remove_path_if_exists(
            &paths.active_workspace_marker_path,
            "active workspace marker",
        )
        .await?;
        remove_dir_if_exists(&paths.current_openhuman_dir, "current openhuman dir").await?;
        if paths.default_openhuman_dir != paths.current_openhuman_dir {
            remove_dir_if_exists(&paths.default_openhuman_dir, "default openhuman dir").await?;
        } else {
            log::debug!(
                "[core] reset_local_data: default dir == current dir; already removed above"
            );
        }
        Ok(())
    }
    .await;
    if let Err(ref e) = delete_result {
        log::warn!("[core] reset_local_data: delete step failed: {e}; will still restart core");
    }

    // ── 5. Restart the embedded core ────────────────────────────────────
    //
    // Always attempt restart, even if delete failed — otherwise the user
    // is left with a dead sidecar. If restart itself fails, prefer the
    // original delete error (more actionable) over the restart error.
    let restart_result = state.inner().ensure_running().await;
    match (&delete_result, &restart_result) {
        (Ok(()), Ok(())) => log::info!("[core] reset_local_data: embedded core back up"),
        (Err(_), Ok(())) => log::warn!(
            "[core] reset_local_data: core restarted but delete step failed; surfacing delete error"
        ),
        (Ok(()), Err(e)) => log::error!("[core] reset_local_data: core restart failed: {e}"),
        (Err(_), Err(e)) => log::error!(
            "[core] reset_local_data: both delete and restart failed; restart error: {e}"
        ),
    }
    delete_result?;
    restart_result?;
    Ok(())
}

/// Resolved data paths returned by `config_get_data_paths`.
struct ResolvedDataPaths {
    current_openhuman_dir: std::path::PathBuf,
    default_openhuman_dir: std::path::PathBuf,
    active_workspace_marker_path: std::path::PathBuf,
}

fn is_windows_file_lock_raw_os_error(raw_os_error: Option<i32>) -> bool {
    matches!(raw_os_error, Some(32 | 33))
}

fn is_windows_file_lock_error(error: &std::io::Error) -> bool {
    cfg!(windows) && is_windows_file_lock_raw_os_error(error.raw_os_error())
}

/// Returns:
///   * `Ok(())` — the underlying remove failure should be swallowed (e.g.
///     the path disappeared between the failed `remove_*` call and the
///     reboot-fallback walk, so there is nothing left to clean up).
///   * `Err(msg)` — a user-facing failure message the caller should surface
///     to the UI / propagate up the reset flow.
fn reset_local_data_delete_error(
    label: &str,
    path: &std::path::Path,
    error: &std::io::Error,
) -> Result<(), String> {
    if is_windows_file_lock_error(error) {
        log::warn!(
            "[core] reset_local_data: Windows file lock blocked removal of {label} at {}: {error}",
            path.display()
        );

        // Fallback: queue the still-locked sub-tree for deletion on the
        // next Windows boot via MoveFileExW + MOVEFILE_DELAY_UNTIL_REBOOT.
        // By this point in `reset_local_data` we have already:
        //   * shut down the embedded core (drops every SQLite/log handle
        //     the core task held), and
        //   * released the host-process log appender via
        //     `shutdown_file_guard()` (drops the rolling log file handle).
        // So any remaining lock now comes from *outside* this process —
        // anti-virus / file indexer / sibling app / Explorer — and cannot
        // be released by closing more OpenHuman windows. See issue #1615.
        #[cfg(target_os = "windows")]
        {
            return schedule_reboot_delete_or_describe(label, path, error);
        }
        // `is_windows_file_lock_error` is gated on `cfg!(windows)`, so on
        // Linux/macOS this branch is unreachable at runtime — but cargo
        // still type-checks the file for those targets and needs a value
        // of type `String`.
        #[cfg(not(target_os = "windows"))]
        {
            return Err(format!(
                "Failed to remove {label} at {} because it is locked by another OpenHuman window or process. Close all OpenHuman windows and try again. ({error})",
                path.display()
            ));
        }
    }

    Err(format!(
        "Failed to remove {label} at {}: {error}",
        path.display()
    ))
}

/// Windows-only: ask the session manager to delete `path` (and its
/// children if it is a directory) on the next reboot, and return either a
/// user-facing message describing the outcome or `Ok(())` when the
/// underlying failure should be treated as already-cleaned-up.
#[cfg(target_os = "windows")]
fn schedule_reboot_delete_or_describe(
    label: &str,
    path: &std::path::Path,
    original_error: &std::io::Error,
) -> Result<(), String> {
    match reset_reboot_schedule::schedule_path_for_reboot_deletion(path) {
        Ok(summary) => {
            log::info!(
                "[core] reset_local_data: scheduled {label} at {} for reboot deletion (files={}, dirs={})",
                path.display(),
                summary.files,
                summary.dirs
            );
            Err(format!(
                "Couldn't remove {label} at {} right now because another process is holding it open ({original_error}). {} files and {} folders have been queued for deletion the next time you restart Windows — restart soon to finish the reset.",
                path.display(),
                summary.files,
                summary.dirs,
            ))
        }
        // Race condition: the still-locked path disappeared between the
        // `remove_*` call that failed with `ERROR_SHARING_VIOLATION` and
        // the metadata read inside the reboot-schedule walk. Whoever else
        // held the handle has already finished cleaning up, so the reset
        // goal is achieved — swallow the original lock error and treat
        // this as success. The empty partial schedule (no entries queued
        // yet) is what distinguishes "vanished cleanly" from "started
        // walking, then hit a real error."
        Err(failure)
            if failure.error.kind() == std::io::ErrorKind::NotFound
                && failure.partial.total() == 0 =>
        {
            log::info!(
                "[core] reset_local_data: {label} at {} disappeared between lock failure and reboot fallback; treating as removed",
                path.display(),
            );
            Ok(())
        }
        Err(failure) => {
            let partial_total = failure.partial.total();
            log::error!(
                "[core] reset_local_data: reboot delete fallback failed for {label} at {}: {} (partial schedule: files={}, dirs={})",
                path.display(),
                failure.error,
                failure.partial.files,
                failure.partial.dirs,
            );
            if partial_total == 0 {
                Err(format!(
                    "Failed to remove {label} at {} because it is locked by another OpenHuman window or process, and scheduling deletion on next reboot also failed ({}). Close all OpenHuman windows and try again. ({original_error})",
                    path.display(),
                    failure.error,
                ))
            } else {
                Err(format!(
                    "Failed to remove {label} at {} because it is locked by another OpenHuman window or process. {} files and {} folders were queued for the next reboot before scheduling failed ({}); the rest still needs manual cleanup. Close all OpenHuman windows and try again. ({original_error})",
                    path.display(),
                    failure.partial.files,
                    failure.partial.dirs,
                    failure.error,
                ))
            }
        }
    }
}

/// Call the core's `config_get_data_paths` RPC and parse the response.
async fn fetch_data_paths() -> Result<ResolvedDataPaths, String> {
    let url = crate::core_rpc::core_rpc_url_value();
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.config_get_data_paths",
        "params": {}
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("config_get_data_paths client build failed: {e}"))?;
    let req = crate::core_rpc::apply_auth(client.post(&url))?;
    let res = req
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("config_get_data_paths request failed: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("config_get_data_paths http {}", res.status()));
    }
    let envelope: serde_json::Value = res
        .json()
        .await
        .map_err(|e| format!("config_get_data_paths decode failed: {e}"))?;
    // JSON-RPC envelope wraps the `RpcOutcome` result twice:
    // `{ "result": { "result": { ...paths... }, "logs": [...] } }`.
    let inner = envelope
        .pointer("/result/result")
        .ok_or_else(|| "config_get_data_paths missing /result/result".to_string())?;
    let current = inner
        .get("current_openhuman_dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "config_get_data_paths missing current_openhuman_dir".to_string())?;
    let default = inner
        .get("default_openhuman_dir")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "config_get_data_paths missing default_openhuman_dir".to_string())?;
    let marker = inner
        .get("active_workspace_marker_path")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "config_get_data_paths missing active_workspace_marker_path".to_string())?;
    Ok(ResolvedDataPaths {
        current_openhuman_dir: std::path::PathBuf::from(current),
        default_openhuman_dir: std::path::PathBuf::from(default),
        active_workspace_marker_path: std::path::PathBuf::from(marker),
    })
}

/// Remove a regular file if present. Missing → debug log + Ok.
async fn remove_path_if_exists(path: &std::path::Path, label: &str) -> Result<(), String> {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {
            log::info!(
                "[core] reset_local_data: removed {label} at {}",
                path.display()
            );
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "[core] reset_local_data: {label} already absent at {}",
                path.display()
            );
            Ok(())
        }
        Err(e) => reset_local_data_delete_error(label, path, &e),
    }
}

/// Remove a directory tree if present. Missing → debug log + Ok.
async fn remove_dir_if_exists(path: &std::path::Path, label: &str) -> Result<(), String> {
    match tokio::fs::remove_dir_all(path).await {
        Ok(()) => {
            log::info!(
                "[core] reset_local_data: removed {label} at {}",
                path.display()
            );
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            log::debug!(
                "[core] reset_local_data: {label} already absent at {}",
                path.display()
            );
            Ok(())
        }
        Err(e) => reset_local_data_delete_error(label, path, &e),
    }
}

#[cfg(test)]
#[path = "local_data_reset_tests.rs"]
mod tests;
