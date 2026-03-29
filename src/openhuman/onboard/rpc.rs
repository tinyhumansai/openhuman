//! JSON-RPC / CLI controller surface for onboarding flows.

use crate::openhuman::config::Config;
use crate::openhuman::onboard::{self, ModelRefreshResult};
use crate::openhuman::rpc::RpcOutcome;

pub async fn models_refresh(
    config: &Config,
    provider_override: Option<&str>,
    force: bool,
) -> Result<RpcOutcome<ModelRefreshResult>, String> {
    let result =
        onboard::run_models_refresh(config, provider_override, force).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(result, "model refresh completed"))
}
