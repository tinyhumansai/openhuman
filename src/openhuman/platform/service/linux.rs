//! systemd user unit install/start/stop/status for Linux.

use crate::openhuman::config::Config;
use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
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
    let _ = run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
    Ok(ServiceStatus {
        state: ServiceState::NotInstalled,
        unit_path: Some(file),
        label: "openhuman.service".to_string(),
        details: None,
    })
}

pub(crate) fn linux_service_file(config: &Config) -> Result<PathBuf> {
    let config_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from);

    Ok(config_dir
        .join(".config")
        .join("systemd")
        .join("user")
        .join("openhuman.service"))
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
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    #[test]
    fn linux_service_file_uses_config_dir() {
        let config = Config::default();
        let path = linux_service_file(&config).unwrap();
        assert!(path.ends_with(".config/systemd/user/openhuman.service"));
    }
}
