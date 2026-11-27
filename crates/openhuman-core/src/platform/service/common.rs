//! Shared helpers for platform service install/lifecycle (all OS targets).

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub(crate) const SERVICE_LABEL: &str = "com.openhuman.core";
pub(crate) const LEGACY_SERVICE_LABEL: &str = "com.openhuman.daemon";
pub(crate) const LEGACY_APP_LABEL: &str = "com.openhuman.app";

pub(crate) fn resolve_daemon_executable() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("OPENHUMAN_CORE_BIN") {
        let candidate = PathBuf::from(path);
        if candidate.exists() {
            return Ok(candidate);
        }
    }

    let exe = std::env::current_exe().context("Failed to resolve current executable")?;
    let exe_dir = exe
        .parent()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve executable directory"))?;

    #[cfg(target_os = "macos")]
    let mut search_dirs = vec![
        exe_dir.clone(),
        exe_dir
            .parent()
            .map(|p| p.join("Resources"))
            .unwrap_or_else(|| exe_dir.clone()),
    ];
    #[cfg(not(target_os = "macos"))]
    let mut search_dirs = vec![exe_dir.clone()];

    for dir in search_dirs.drain(..) {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() || is_current_executable(&path) {
                continue;
            }
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            #[cfg(windows)]
            let matches = name.starts_with("openhuman-core-")
                || name.eq_ignore_ascii_case("openhuman-core.exe");
            #[cfg(not(windows))]
            let matches = name.starts_with("openhuman-core-") || name == "openhuman-core";

            if matches {
                return Ok(path);
            }
        }
    }

    Ok(exe)
}

pub(crate) fn daemon_program_args(_exe: &std::path::Path) -> Vec<String> {
    vec!["run".to_string()]
}

fn is_current_executable(candidate: &std::path::Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    same_executable_path(candidate, &current)
}

fn same_executable_path(a: &std::path::Path, b: &std::path::Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a_real), Ok(b_real)) => a_real == b_real,
        _ => false,
    }
}

pub(crate) fn daemon_command_line(exe: &std::path::Path) -> String {
    let args = daemon_program_args(exe);
    let exe_quoted = format!("\"{}\"", exe.display());
    if args.is_empty() {
        exe_quoted
    } else {
        format!("{} {}", exe_quoted, args.join(" "))
    }
}

pub(crate) fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Suppress conhost allocation for Windows command spawns. Without this,
/// every `schtasks /Query` polled by service status checks flashes a
/// console — visible to users (#1475 follow-up to #731 + #1338).
#[cfg(windows)]
fn no_window(cmd: &mut Command) {
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
fn no_window(_cmd: &mut Command) {}

pub(crate) fn run_checked(cmd: &mut Command) -> Result<()> {
    no_window(cmd);
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("command failed with status {status}");
    }
    Ok(())
}

pub(crate) fn run_capture(cmd: &mut Command) -> Result<String> {
    no_window(cmd);
    let output = cmd.output()?;
    if !output.status.success() {
        anyhow::bail!("command failed with status {}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) fn run_best_effort(cmd: &mut Command) {
    no_window(cmd);
    match cmd.stdout(Stdio::null()).stderr(Stdio::null()).status() {
        Ok(status) => {
            if !status.success() {
                log::debug!("[service] best-effort command failed with status {status}");
            }
        }
        Err(err) => {
            log::debug!("[service] best-effort command failed to execute: {err}");
        }
    }
}

pub(crate) fn run_check_silent(cmd: &mut Command) -> bool {
    no_window(cmd);
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// File name of the Linux systemd user unit.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) const SYSTEMD_USER_UNIT_NAME: &str = "openhuman.service";

/// Directory systemd loads per-user units from: `$XDG_CONFIG_HOME/systemd/user`,
/// falling back to `$HOME/.config/systemd/user`.
///
/// Per the XDG Base Directory spec an empty or relative `XDG_CONFIG_HOME` is
/// ignored (this matches `dirs::config_dir()` on Linux). Returns `None` when
/// neither a usable `XDG_CONFIG_HOME` nor a home directory is available.
///
/// Pure (no environment access) and not cfg-gated so it is unit-tested on every
/// OS; the Linux backend feeds it from the process environment.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn systemd_user_unit_dir(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::path::Path>,
) -> Option<PathBuf> {
    let config_home = xdg_config_home
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            home.filter(|h| !h.as_os_str().is_empty())
                .map(|h| h.join(".config"))
        })?;
    Some(config_home.join("systemd").join("user"))
}

#[cfg(test)]
#[path = "common_tests.rs"]
mod tests;
