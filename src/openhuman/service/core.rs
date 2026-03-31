#[cfg(target_os = "linux")]
use super::linux;
#[cfg(target_os = "macos")]
use super::macos;
#[cfg(windows)]
use super::windows;
use crate::openhuman::config::Config;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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

pub fn install(config: &Config) -> Result<ServiceStatus> {
    if super::mock::is_enabled() {
        return super::mock::install(config);
    }

    #[cfg(target_os = "macos")]
    {
        macos::install(config)?;
        status(config)
    }
    #[cfg(target_os = "linux")]
    {
        linux::install(config)?;
        return status(config);
    }
    #[cfg(windows)]
    {
        windows::install(config)?;
        return status(config);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn start(config: &Config) -> Result<ServiceStatus> {
    if super::mock::is_enabled() {
        return super::mock::start(config);
    }

    #[cfg(target_os = "macos")]
    return macos::start(config);
    #[cfg(target_os = "linux")]
    return linux::start(config);
    #[cfg(windows)]
    return windows::start(config);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn stop(config: &Config) -> Result<ServiceStatus> {
    if super::mock::is_enabled() {
        return super::mock::stop(config);
    }

    #[cfg(target_os = "macos")]
    {
        macos::stop(config)?;
        status(config)
    }
    #[cfg(target_os = "linux")]
    {
        linux::stop(config)?;
        return status(config);
    }
    #[cfg(windows)]
    {
        windows::stop(config)?;
        return status(config);
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn status(config: &Config) -> Result<ServiceStatus> {
    if super::mock::is_enabled() {
        return super::mock::status(config);
    }

    #[cfg(target_os = "macos")]
    return macos::status(config);
    #[cfg(target_os = "linux")]
    return linux::status(config);
    #[cfg(windows)]
    return windows::status(config);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}

pub fn uninstall(config: &Config) -> Result<ServiceStatus> {
    if super::mock::is_enabled() {
        return super::mock::uninstall(config);
    }

    let _ = stop(config);

    #[cfg(target_os = "macos")]
    return macos::uninstall(config);
    #[cfg(target_os = "linux")]
    return linux::uninstall(config);
    #[cfg(windows)]
    return windows::uninstall(config);
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    anyhow::bail!("Service management is supported on macOS, Linux, and Windows only")
}
