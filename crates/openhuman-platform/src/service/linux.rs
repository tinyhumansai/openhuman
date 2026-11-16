//! systemd user unit install/start/stop/status for Linux.

use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use super::common::{run_capture, run_checked, systemd_command_line};
use super::{ServiceSpec, ServiceState, ServiceStatus};

pub(crate) fn install(spec: &ServiceSpec) -> Result<()> {
    let file = linux_service_file(spec);
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent)?;
    }

    let logs_dir = &spec.logs_dir;
    fs::create_dir_all(logs_dir)?;

    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");
    let exec_start = systemd_command_line(&spec.exe, &spec.args)?;

    let unit = format!(
        "[Unit]\nDescription=OpenHuman Daemon\n\n[Service]\nExecStart={}\nRestart=always\nRestartSec=3\n\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
        exec_start,
        stdout.display(),
        stderr.display(),
    );

    fs::write(&file, unit)?;
    let _ = run_checked(Command::new("systemctl").args(["--user", "enable", spec.label.as_str()]));
    Ok(())
}

pub(crate) fn start(spec: &ServiceSpec) -> Result<ServiceStatus> {
    if !is_service_enabled_linux(spec)? {
        log::info!("[service] Enabling systemd service");
        let _ =
            run_checked(Command::new("systemctl").args(["--user", "enable", spec.label.as_str()]));
    } else {
        log::info!("[service] Systemd service already enabled");
    }

    run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]))?;

    log::info!("[service] Starting systemd service");
    let start_result =
        run_checked(Command::new("systemctl").args(["--user", "start", spec.label.as_str()]));
    if let Err(e) = start_result {
        let status_check = super::status(spec)?;
        if matches!(status_check.state, ServiceState::Running) {
            log::info!("[service] Service was already running - operation successful");
        } else {
            return Err(e);
        }
    }
    super::status(spec)
}

pub(crate) fn stop(spec: &ServiceSpec) -> Result<()> {
    let _ = run_checked(Command::new("systemctl").args(["--user", "stop", spec.label.as_str()]));
    Ok(())
}

pub(crate) fn status(spec: &ServiceSpec) -> Result<ServiceStatus> {
    let out =
        run_capture(Command::new("systemctl").args(["--user", "is-active", spec.label.as_str()]))
            .unwrap_or_else(|_| "unknown".into());
    let state = match out.trim() {
        "active" => ServiceState::Running,
        "inactive" | "failed" => ServiceState::Stopped,
        other => ServiceState::Unknown(other.to_string()),
    };
    Ok(ServiceStatus {
        state,
        unit_path: Some(linux_service_file(spec)),
        label: spec.label.clone(),
        details: None,
    })
}

pub(crate) fn uninstall(spec: &ServiceSpec) -> Result<ServiceStatus> {
    let file = linux_service_file(spec);
    if file.exists() {
        fs::remove_file(&file).with_context(|| format!("Failed to remove {}", file.display()))?;
    }
    let _ = run_checked(Command::new("systemctl").args(["--user", "daemon-reload"]));
    Ok(ServiceStatus {
        state: ServiceState::NotInstalled,
        unit_path: Some(file),
        label: spec.label.clone(),
        details: None,
    })
}

/// `<work_dir>/.config/systemd/user/<label>`.
pub(crate) fn linux_service_file(spec: &ServiceSpec) -> PathBuf {
    user_unit_path(&spec.work_dir, &spec.label)
}

/// Where the user unit named `label` lives for a service running in
/// `work_dir`. Shared with the mock so both report the same `unit_path`.
pub(crate) fn user_unit_path(work_dir: &std::path::Path, label: &str) -> PathBuf {
    work_dir
        .join(".config")
        .join("systemd")
        .join("user")
        .join(label)
}

fn is_service_enabled_linux(spec: &ServiceSpec) -> Result<bool> {
    let result = Command::new("systemctl")
        .args(["--user", "is-enabled", spec.label.as_str()])
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
