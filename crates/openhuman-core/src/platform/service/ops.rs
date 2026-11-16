//! JSON-RPC / CLI controller surface for platform service install/lifecycle.

use crate::config::Config;
use crate::core::Outcome;
use crate::platform::service::daemon_host::DaemonHostConfig;
use crate::platform::service::{self, daemon_host};
use openhuman_platform::service::ServiceStatus;

/// Installs the OpenHuman daemon as a system service.
pub async fn service_install(config: &Config) -> Result<Outcome<ServiceStatus>, String> {
    let status = service::install(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(status, "service install completed"))
}

/// Starts the installed OpenHuman daemon service.
pub async fn service_start(config: &Config) -> Result<Outcome<ServiceStatus>, String> {
    let status = service::start(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(status, "service start completed"))
}

/// Stops the running OpenHuman daemon service.
pub async fn service_stop(config: &Config) -> Result<Outcome<ServiceStatus>, String> {
    let status = service::stop(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(status, "service stop completed"))
}

/// Returns the current status of the OpenHuman daemon service.
pub async fn service_status(config: &Config) -> Result<Outcome<ServiceStatus>, String> {
    let status = service::status(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(status, "service status fetched"))
}

/// Requests an asynchronous restart of the core process.
pub async fn service_restart(
    source: Option<String>,
    reason: Option<String>,
) -> Result<Outcome<service::RestartStatus>, String> {
    service::restart::service_restart(source, reason).await
}

/// Requests an asynchronous graceful shutdown of the core process.
pub async fn service_shutdown(
    source: Option<String>,
    reason: Option<String>,
) -> Result<Outcome<service::ShutdownStatus>, String> {
    service::shutdown::service_shutdown(source, reason).await
}

/// Uninstalls the OpenHuman daemon system service.
pub async fn service_uninstall(config: &Config) -> Result<Outcome<ServiceStatus>, String> {
    let status = service::uninstall(config).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(status, "service uninstall completed"))
}

/// Reads the daemon host UI preferences from the configuration directory.
pub async fn daemon_host_get(config: &Config) -> Result<Outcome<DaemonHostConfig>, String> {
    let config_dir = config
        .config_path
        .parent()
        .ok_or_else(|| "failed to resolve config directory".to_string())?;
    let current = daemon_host::load_for_config_dir(config_dir).await;
    Ok(Outcome::single_log(current, "daemon host config loaded"))
}

/// Updates the daemon host UI preferences and saves them to disk.
pub async fn daemon_host_set(
    config: &Config,
    show_tray: bool,
) -> Result<Outcome<DaemonHostConfig>, String> {
    let config_dir = config
        .config_path
        .parent()
        .ok_or_else(|| "failed to resolve config directory".to_string())?;
    let next = DaemonHostConfig { show_tray };
    daemon_host::save_for_config_dir(config_dir, &next).await?;
    Ok(Outcome::single_log(next, "daemon host config saved"))
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
