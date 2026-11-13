//! Tauri commands for the direct-CLI service fallback.
//!
//! The frontend reaches the daemon service through the core RPC first
//! (`openhuman.service_install` and friends). When that path is unavailable —
//! the embedded core is not running yet, or the renderer is talking to a
//! remote runtime — `tauriCommands/service.ts` falls back to these commands,
//! which run the same operation as a one-shot `openhuman-core service …` CLI
//! invocation and hand the raw stdout back to the renderer.
//!
//! The CLI prints the controller's JSON envelope on stdout, which is exactly
//! the `{ result, logs }` shape the frontend's `parseServiceCliOutput`
//! expects, so the payload is passed through verbatim.
//!
//! These commands were missing on the Rust side: the frontend invoked
//! `service_install_direct` and friends, but no such command was registered,
//! so every fallback invoke failed with "Command … not found".

use std::path::Path;
use std::process::Command;

use crate::core_binary::resolve_core_binary;

/// Run a command, return its stdout, and map a non-zero exit to an error
/// carrying the command's stderr.
///
/// Kept separate from the service wiring so the exit-status handling can be
/// exercised without a real `openhuman-core` binary.
fn run_capture(binary: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(binary)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run {}: {e}", binary.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            format!(
                "{} exited with status {}",
                binary.display(),
                output.status
            )
        } else {
            stderr
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Run `openhuman-core service <verb>` and return its stdout.
///
/// A non-zero exit is surfaced as an error carrying the CLI's stderr, which
/// is where the core reports why the operation failed (permission denied,
/// the service is not installed, …).
fn run_service_cli(verb: &str) -> Result<String, String> {
    let exe = resolve_core_binary()?;

    log::debug!(
        "[service_commands] running {} service {verb}",
        exe.display()
    );

    run_capture(&exe, &["service", verb])
}

/// Install the OpenHuman daemon as a system service.
#[tauri::command]
pub fn service_install_direct() -> Result<String, String> {
    run_service_cli("install")
}

/// Start the installed OpenHuman daemon service.
#[tauri::command]
pub fn service_start_direct() -> Result<String, String> {
    run_service_cli("start")
}

/// Stop the running OpenHuman daemon service.
#[tauri::command]
pub fn service_stop_direct() -> Result<String, String> {
    run_service_cli("stop")
}

/// Report the current daemon/service status.
#[tauri::command]
pub fn service_status_direct() -> Result<String, String> {
    run_service_cli("status")
}

/// Uninstall the OpenHuman daemon system service.
#[tauri::command]
pub fn service_uninstall_direct() -> Result<String, String> {
    run_service_cli("uninstall")
}

#[cfg(test)]
#[path = "service_commands_tests.rs"]
mod tests;
