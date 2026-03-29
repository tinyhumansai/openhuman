//! JSON-RPC / CLI controller surface for platform service install/lifecycle.

use crate::openhuman::config::Config;
use crate::openhuman::service::{self, ServiceStatus};
use crate::rpc::RpcOutcome;

pub async fn service_install(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::install(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service install completed"))
}

pub async fn service_start(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::start(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service start completed"))
}

pub async fn service_stop(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::stop(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service stop completed"))
}

pub async fn service_status(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::status(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(status, "service status fetched"))
}

pub async fn service_uninstall(config: &Config) -> Result<RpcOutcome<ServiceStatus>, String> {
    let status = service::uninstall(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        status,
        "service uninstall completed",
    ))
}
