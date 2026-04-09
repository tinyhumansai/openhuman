//! JSON-RPC / CLI controller surface for platform service install/lifecycle.

use crate::openhuman::config::Config;
use crate::openhuman::service::daemon_host::DaemonHostConfig;
use crate::openhuman::service::{self, daemon_host, ServiceStatus};
use crate::rpc::RpcOutcome;

/// Installs the OpenHuman daemon as a system service.
pub async fn service_install(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::install(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service install completed"))
}

/// Starts the installed OpenHuman daemon service.
pub async fn service_start(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::start(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service start completed"))
}

/// Stops the running OpenHuman daemon service.
pub async fn service_stop(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::stop(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service stop completed"))
}

/// Returns the current status of the OpenHuman daemon service.
pub async fn service_status(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::status(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service status fetched"))
}

/// Requests an asynchronous restart of the core process.
pub async fn service_restart(
    source: Option<String>,
    reason: Option<String>,
) -> Result<RpcOutcome<service::RestartStatus>, String> {
    service::restart::service_restart(source, reason).await
}

/// Uninstalls the OpenHuman daemon system service.
pub async fn service_uninstall(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::uninstall(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        status,
        "service uninstall completed",
    ))
}

/// Reads the daemon host UI preferences from the configuration directory.
pub async fn daemon_host_get(config: &Config) -> Result<RpcOutcome<DaemonHostConfig>, String> {
    let config_dir = config
        .config_path
        .parent()
        .ok_or_else(|| "failed to resolve config directory".to_string())?;
    let current = daemon_host::load_for_config_dir(config_dir).await;
    Ok(RpcOutcome::single_log(current, "daemon host config loaded"))
}

/// Updates the daemon host UI preferences and saves them to disk.
pub async fn daemon_host_set(
    config: &Config,
    show_tray: bool,
) -> Result<RpcOutcome<DaemonHostConfig>, String> {
    let config_dir = config
        .config_path
        .parent()
        .ok_or_else(|| "failed to resolve config directory".to_string())?;
    let next = DaemonHostConfig { show_tray };
    daemon_host::save_for_config_dir(config_dir, &next).await?;
    Ok(RpcOutcome::single_log(next, "daemon host config saved"))
}
