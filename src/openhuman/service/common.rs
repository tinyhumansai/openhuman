//! Shared helpers for platform service install/lifecycle (all OS targets).

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

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

pub(crate) fn run_checked(cmd: &mut Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        anyhow::bail!("command failed with status {status}");
    }
    Ok(())
}

pub(crate) fn run_capture(cmd: &mut Command) -> Result<String> {
    let output = cmd.output()?;
    if !output.status.success() {
        anyhow::bail!("command failed with status {}", output.status);
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

pub(crate) fn run_best_effort(cmd: &mut Command) {
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
    cmd.stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_escape_replaces_entities() {
        let raw = "<tag>\"&'";
        let escaped = xml_escape(raw);
        assert!(escaped.contains("&lt;tag&gt;"));
        assert!(escaped.contains("&quot;"));
        assert!(escaped.contains("&amp;"));
        assert!(escaped.contains("&apos;"));
    }
}
