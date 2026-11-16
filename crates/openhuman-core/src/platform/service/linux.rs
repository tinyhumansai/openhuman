//! systemd user unit install/start/stop/status for Linux.

use crate::config::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::common::{self, run_capture, run_checked};
use super::{ServiceState, ServiceStatus};

pub(crate) fn install(config: &Config) -> Result<()> {
    let file = linux_service_file(config)?;
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = common::resolve_daemon_executable()?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");
    let exec_start = common::daemon_command_line(&exe);

    let unit = format!(
        "[Unit]\nDescription=OpenHuman Daemon\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=3\n\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
        exec_start,
        stdout.display(),
        stderr.display(),
    );

    fs::write(&file, unit)?;
    remove_legacy_service_file(config, &file);
    let _ = run_checked(Command::new("systemctl").args(["--user", "enable", "openhuman.service"]));
    Ok(())
}

pub(crate) fn start(config: &Config) -> Result<ServiceStatus> {
    if !is_service_enabled_linux()? {
        log::info!("[service] Enabling systemd service");
        let _ =
            run_checked(Command::new("systemctl").args(["--user", "enable", "openhuman.service"]));
    } else {
        log::info!("[service] Systemd service already enabled");
    }

    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;

    log::info!("[service] Starting systemd service");
    let start_result =
        run_checked(Command::new("systemctl").args(["--user", "start", "openhuman.service"]));
    if let Err(e) = start_result {
        let status_check = super::status(config)?;
        if matches!(status_check.state, ServiceState::Running) {
            log::info!("[service] Service was already running - operation successful");
        } else {
            return Err(e);
        }
    }
    super::status(config)
}

pub(crate) fn stop(_config: &Config) -> Result<()> {
    let _ = run_checked(Command::new("systemctl").args(["--user", "stop", "openhuman.service"]));
    Ok(())
}

pub(crate) fn status(config: &Config) -> Result<ServiceStatus> {
    let out =
        run_capture(Command::new("systemctl").args(["--user", "is-active", "openhuman.service"]))
            .unwrap_or_else(|_| "unknown".into());
    let state = match out.trim() {
        "active" => ServiceState::Running,
        "inactive" | "failed" => ServiceState::Stopped,
        other => ServiceState::Unknown(other.to_string()),
    };
    Ok(ServiceStatus {
        state,
        unit_path: Some(linux_service_file(config)?),
        label: "openhuman.service".to_string(),
        details: None,
    })
}

pub(crate) fn uninstall(config: &Config) -> Result<ServiceStatus> {
    let file = linux_service_file(config)?;
    if file.exists() {
        fs::remove_file(&file).with_context(|| format!("Failed to remove {}", file.display()))?;
    }
    remove_legacy_service_file(config, &file);
    let _ = run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
    Ok(ServiceStatus {
        state: ServiceState::NotInstalled,
        unit_path: Some(file),
        label: "openhuman.service".to_string(),
        details: None,
    })
}

/// Path of the systemd user unit, in the directory systemd actually loads user
/// units from (`$XDG_CONFIG_HOME/systemd/user` or `~/.config/systemd/user`).
///
/// Shared by `install`, `status` and `uninstall` so they always agree.
pub(crate) fn linux_service_file(_config: &Config) -> Result<PathBuf> {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
    let home = dirs::home_dir();
    let dir = common::systemd_user_unit_dir(xdg_config_home.as_deref(), home.as_deref())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Cannot resolve the systemd user unit directory: XDG_CONFIG_HOME is unset, \
                 empty or relative and no home directory is available"
            )
        })?;
    Ok(dir.join(common::SYSTEMD_USER_UNIT_NAME))
}

/// Where releases before this fix wrote the unit (under the OpenHuman config
/// dir, e.g. `~/.openhuman/.config/systemd/user/openhuman.service`). systemd
/// never loaded it from there; it is only kept to clean that file up.
fn legacy_linux_service_file(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join(".config")
        .join("systemd")
        .join("user")
        .join(common::SYSTEMD_USER_UNIT_NAME)
}

/// Best-effort removal of the stale unit left by older releases. Only that
/// exact file is touched, and never when it coincides with the real unit path.
fn remove_legacy_service_file(config: &Config, current: &Path) {
    let legacy = legacy_linux_service_file(config);
    if legacy == current || !legacy.is_file() {
        return;
    }
    // Same file reached through a symlink (e.g. the config dir links to $HOME).
    if let (Ok(a), Ok(b)) = (fs::canonicalize(&legacy), fs::canonicalize(current)) {
        if a == b {
            return;
        }
    }
    match fs::remove_file(&legacy) {
        Ok(()) => log::info!(
            "[service] Removed stale systemd unit from legacy location {}",
            legacy.display()
        ),
        Err(e) => log::warn!(
            "[service] Failed to remove stale systemd unit {}: {e}",
            legacy.display()
        ),
    }
}

fn is_service_enabled_linux() -> Result<bool> {
    let result = Command::new("systemctl")
        .args(["--user", "is-enabled", "openhuman.service"])
        .output();

    match result {
        Ok(output) => {
            let status_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
            Ok(status_str == "enabled")
        }
        Err(_) => Ok(false),
    }
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
