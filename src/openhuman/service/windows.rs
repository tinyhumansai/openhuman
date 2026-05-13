//! Scheduled task install/start/stop/status for Windows.

use crate::openhuman::config::Config;
use anyhow::Result;
use std::fs;
// File is `#[cfg(windows)]`-gated at the module level — no per-item guard needed.
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use super::common::{self, run_capture, run_checked};
use super::{ServiceState, ServiceStatus};

const WINDOWS_TASK_NAME: &str = "OpenHuman Core";

fn windows_task_name() -> &'static str {
    WINDOWS_TASK_NAME
}

pub(crate) fn install(config: &Config) -> Result<()> {
    let exe = common::resolve_daemon_executable()?;
    let logs_dir = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs");
    fs::create_dir_all(&logs_dir)?;

    let wrapper = logs_dir.join("openhuman-daemon.cmd");
    let stdout = logs_dir.join("daemon.stdout.log");
    let stderr = logs_dir.join("daemon.stderr.log");
    let daemon_cmd = common::daemon_command_line(&exe);

    let cmd = format!(
        "@echo off\n{} >> \"{}\" 2>> \"{}\"\n",
        daemon_cmd,
        stdout.display(),
        stderr.display()
    );
    fs::write(&wrapper, cmd)?;

    run_checked(Command::new("schtasks").args([
        "/Create",
        "/TN",
        windows_task_name(),
        "/TR",
        &wrapper.display().to_string(),
        "/SC",
        "ONLOGON",
        "/F",
    ]))?;

    Ok(())
}

pub(crate) fn start(config: &Config) -> Result<ServiceStatus> {
    let task_name = windows_task_name();

    if !is_task_exists_windows(task_name)? {
        log::warn!("[service] Windows scheduled task does not exist, please install first");
        return Ok(ServiceStatus {
            state: ServiceState::NotInstalled,
            unit_path: None,
            label: task_name.to_string(),
            details: Some("Task not installed".to_string()),
        });
    }

    log::info!("[service] Starting Windows scheduled task");
    let run_result = run_checked(Command::new("schtasks").args(["/Run", "/TN", task_name]));
    if let Err(e) = run_result {
        let status_check = super::status(config)?;
        if matches!(status_check.state, ServiceState::Running) {
            log::info!("[service] Task was already running - operation successful");
            return Ok(status_check);
        }
        return Err(e);
    }
    super::status(config)
}

pub(crate) fn stop(_config: &Config) -> Result<()> {
    let task_name = windows_task_name();
    let _ = run_checked(Command::new("schtasks").args(["/End", "/TN", task_name]));
    Ok(())
}

pub(crate) fn status(_config: &Config) -> Result<ServiceStatus> {
    let task_name = windows_task_name();
    let out =
        run_capture(Command::new("schtasks").args(["/Query", "/TN", task_name, "/FO", "LIST"]));
    match out {
        Ok(text) => {
            let running = text.contains("Running");
            Ok(ServiceStatus {
                state: if running {
                    ServiceState::Running
                } else {
                    ServiceState::Stopped
                },
                unit_path: None,
                label: task_name.to_string(),
                details: None,
            })
        }
        Err(err) => Ok(ServiceStatus {
            state: ServiceState::NotInstalled,
            unit_path: None,
            label: task_name.to_string(),
            details: Some(err.to_string()),
        }),
    }
}

pub(crate) fn uninstall(config: &Config) -> Result<ServiceStatus> {
    let task_name = windows_task_name();
    let _ = run_checked(Command::new("schtasks").args(["/Delete", "/TN", task_name, "/F"]));
    let wrapper = config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
        .join("logs")
        .join("openhuman-daemon.cmd");
    if wrapper.exists() {
        fs::remove_file(&wrapper).ok();
    }
    Ok(ServiceStatus {
        state: ServiceState::NotInstalled,
        unit_path: None,
        label: task_name.to_string(),
        details: None,
    })
}

fn is_task_exists_windows(task_name: &str) -> Result<bool> {
    let result = Command::new("schtasks")
        .args(["/Query", "/TN", task_name])
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
        .output();

    match result {
        Ok(output) => Ok(output.status.success()),
        Err(_) => Ok(false),
    }
}
