mod ai;
mod core;
mod memory;
mod openhuman;

use crate::core_server::types::{invocation_to_rpc_json, AppState};

pub async fn dispatch(
    state: AppState,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    if let Some(result) = core::try_dispatch(&state, method, params.clone()) {
        return result.map(invocation_to_rpc_json);
    }
    if let Some(result) = memory::try_dispatch(method, params.clone()).await {
        return result.map(invocation_to_rpc_json);
    }
    if let Some(result) = ai::try_dispatch(method, params.clone()).await {
        return result.map(invocation_to_rpc_json);
    }
    if let Some(result) = openhuman::try_dispatch(&state, method, params).await {
        return result.map(invocation_to_rpc_json);
    }
    Err(format!("unknown method: {method}"))
}
