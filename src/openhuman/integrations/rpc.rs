//! JSON-RPC / CLI controller surface for the integration registry.

use crate::openhuman::config::Config;
use crate::openhuman::integrations::{self, IntegrationInfo};
use crate::rpc::RpcOutcome;

pub async fn list_integrations(
    config: &Config,
) -> Result<RpcOutcome<Vec<IntegrationInfo>>, String> {
    Ok(RpcOutcome::single_log(
        integrations::list_integrations(config),
        "integrations listed",
    ))
}

pub async fn get_integration_info(
    config: &Config,
    name: &str,
) -> Result<RpcOutcome<IntegrationInfo>, String> {
    let info = integrations::get_integration_info(config, name).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::new(
        info,
        vec![format!("integration loaded: {name}")],
    ))
}
