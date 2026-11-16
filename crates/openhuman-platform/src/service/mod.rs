//! Per-user OS service managers: launchd (macOS), systemd user units (Linux)
//! and scheduled tasks (Windows), plus the file-backed mock E2E tests drive.
//!
//! Callers describe the service with a [`ServiceSpec`]; nothing here reads
//! product configuration. Every dispatcher routes to the mock first when
//! `OPENHUMAN_SERVICE_MOCK` is set.

mod common;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
mod mock;
#[cfg(windows)]
mod windows;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub use mock::mock_agent_running;

/// What to install and how to name it on this OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSpec {
    /// The OS-level identity: the launchd label (`<label>.plist`), the systemd
    /// unit name, or the scheduled-task name.
    pub label: String,
    /// Older labels the macOS manager also stops, probes and removes, so an
    /// upgrade does not leave a previous install running beside this one.
    /// Ignored on Linux and Windows.
    pub legacy_labels: Vec<String>,
    /// The executable the service launches.
    pub exe: PathBuf,
    /// Arguments passed to `exe`.
    pub args: Vec<String>,
    /// Directory for `daemon.stdout.log` / `daemon.stderr.log` (and, on
    /// Windows, the `openhuman-daemon.cmd` wrapper).
    pub logs_dir: PathBuf,
    /// Directory the service runs in. The Linux user unit is written under
    /// `<work_dir>/.config/systemd/user/`, and the mock keeps its state file
    /// here unless `OPENHUMAN_SERVICE_MOCK_STATE_FILE` names one.
    pub work_dir: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServiceState {
    Running,
    Stopped,
    NotInstalled,
    Unknown(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub state: ServiceState,
    pub unit_path: Option<PathBuf>,
    pub label: String,
    pub details: Option<String>,
}

pub fn install(spec: &ServiceSpec) -> Result<ServiceStatus> {
    if mock::is_enabled() {
        return mock::install(spec);
    }

    #[cfg(target_os = "macos")]
    {
        macos::install(spec)?;
        status(spec)
    }
    #[cfg(target_os = "linux")]
    {
        linux::install(spec)?;
        status(spec)
    }
    #[cfg(windows)]
    {
        windows::install(spec)?;
        status(spec)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn start(spec: &ServiceSpec) -> Result<ServiceStatus> {
    if mock::is_enabled() {
        return mock::start(spec);
    }

    #[cfg(target_os = "macos")]
    return macos::start(spec);
    #[cfg(target_os = "linux")]
    return linux::start(spec);
    #[cfg(windows)]
    return windows::start(spec);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn stop(spec: &ServiceSpec) -> Result<ServiceStatus> {
    if mock::is_enabled() {
        return mock::stop(spec);
    }

    #[cfg(target_os = "macos")]
    {
        macos::stop(spec)?;
        status(spec)
    }
    #[cfg(target_os = "linux")]
    {
        linux::stop(spec)?;
        status(spec)
    }
    #[cfg(windows)]
    {
        windows::stop(spec)?;
        status(spec)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn status(spec: &ServiceSpec) -> Result<ServiceStatus> {
    if mock::is_enabled() {
        return mock::status(spec);
    }

    #[cfg(target_os = "macos")]
    return macos::status(spec);
    #[cfg(target_os = "linux")]
    return linux::status(spec);
    #[cfg(windows)]
    return windows::status(spec);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn uninstall(spec: &ServiceSpec) -> Result<ServiceStatus> {
    if mock::is_enabled() {
        return mock::uninstall(spec);
    }

    let _ = stop(spec);

    #[cfg(target_os = "macos")]
    return macos::uninstall(spec);
    #[cfg(target_os = "linux")]
    return linux::uninstall(spec);
    #[cfg(windows)]
    return windows::uninstall(spec);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}
