//! The core daemon as an OS service: builds the
//! [`ServiceSpec`](openhuman_platform::service::ServiceSpec) the managers in
//! `openhuman-platform` install, and binds their lifecycle calls to a
//! [`Config`].
//!
//! Product identity lives here, not in the managers: the service labels, the
//! daemon executable lookup, its arguments, and where its logs go.

use anyhow::{Context, Result};
use openhuman_platform::service::{self as manager, ServiceSpec, ServiceStatus};
use std::fs;
use std::path::{Path, PathBuf};

use crate::config::Config;

/// The launchd label (macOS), and the label reported on unsupported targets.
const SERVICE_LABEL: &str = "com.openhuman.core";
/// Labels earlier installs used on macOS; stopped and removed beside the
/// current one.
const LEGACY_SERVICE_LABEL: &str = "com.openhuman.daemon";
const LEGACY_APP_LABEL: &str = "com.openhuman.app";
/// The systemd user unit (Linux).
#[cfg(target_os = "linux")]
const LINUX_UNIT: &str = "openhuman.service";
/// The scheduled task (Windows).
#[cfg(windows)]
const WINDOWS_TASK_NAME: &str = "OpenHuman Core";

/// Describe the core daemon for `config`: it runs in the config directory and
/// logs to `<config dir>/logs`.
pub(crate) fn service_spec(config: &Config) -> Result<ServiceSpec> {
    let work_dir = config_dir(config);
    Ok(ServiceSpec {
        label: platform_label().to_string(),
        legacy_labels: if cfg!(target_os = "macos") {
            vec![
                LEGACY_SERVICE_LABEL.to_string(),
                LEGACY_APP_LABEL.to_string(),
            ]
        } else {
            Vec::new()
        },
        exe: resolve_daemon_executable()?,
        args: daemon_program_args(),
        logs_dir: work_dir.join("logs"),
        work_dir,
    })
}

/// Installs the core daemon as a per-user OS service.
pub fn install(config: &Config) -> Result<ServiceStatus> {
    manager::install(&service_spec(config)?)
}

/// Starts the installed core daemon service, installing it first on macOS.
pub fn start(config: &Config) -> Result<ServiceStatus> {
    manager::start(&service_spec(config)?)
}

/// Stops the core daemon service.
pub fn stop(config: &Config) -> Result<ServiceStatus> {
    manager::stop(&service_spec(config)?)
}

/// Reports the core daemon service's state.
pub fn status(config: &Config) -> Result<ServiceStatus> {
    manager::status(&service_spec(config)?)
}

/// Stops and removes the core daemon service.
pub fn uninstall(config: &Config) -> Result<ServiceStatus> {
    manager::uninstall(&service_spec(config)?)
}

fn platform_label() -> &'static str {
    #[cfg(target_os = "linux")]
    return LINUX_UNIT;
    #[cfg(windows)]
    return WINDOWS_TASK_NAME;
    #[cfg(not(any(target_os = "linux", windows)))]
    return SERVICE_LABEL;
}

fn config_dir(config: &Config) -> PathBuf {
    config
        .config_path
        .parent()
        .map_or_else(|| PathBuf::from("."), PathBuf::from)
}

fn daemon_program_args() -> Vec<String> {
    vec!["run".to_string()]
}

fn resolve_daemon_executable() -> Result<PathBuf> {
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

fn is_current_executable(candidate: &Path) -> bool {
    let Ok(current) = std::env::current_exe() else {
        return false;
    };
    same_executable_path(candidate, &current)
}

fn same_executable_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a_real), Ok(b_real)) => a_real == b_real,
        _ => false,
    }
}

#[cfg(test)]
#[path = "spec_tests.rs"]
mod tests;
