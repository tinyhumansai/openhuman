//! JSON-RPC / CLI controller surface for model catalog refresh.

use crate::openhuman::config::Config;
use crate::openhuman::model_catalog::{self, ModelRefreshResult};
use crate::rpc::RpcOutcome;

pub async fn models_refresh(
    config: &Config,
    force: bool,
) -> Result<RpcOutcome<ModelRefreshResult>, String> {
    let result = model_catalog::run_models_refresh(config, force).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(result, "model refresh completed"))
}
