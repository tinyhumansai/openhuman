//! Service management helpers for OpenHuman daemon.

pub mod daemon;
pub mod daemon_host;
pub mod rpc;
mod schemas;
pub use schemas::{
    all_controller_schemas as all_service_controller_schemas,
    all_registered_controllers as all_service_registered_controllers,
};

mod common;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

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
    #[cfg(target_os = "macos")]
    {
        macos::install(config)?;
        return status(config);
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
    #[cfg(target_os = "macos")]
    {
        macos::stop(config)?;
        return status(config);
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
