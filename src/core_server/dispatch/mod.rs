mod core;

use crate::core_server::rpc_log;
use crate::core_server::types::AppState;

pub async fn dispatch(
    state: AppState,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    log::trace!(
        "[rpc:dispatch] enter method={} params={}",
        method,
        rpc_log::redact_params_for_log(&params)
    );

    if let Some(result) = core::try_dispatch(&state, method, params.clone()) {
        log::debug!("[rpc:dispatch] routed method={} subsystem=core", method);
        return result.map(crate::core_server::types::invocation_to_rpc_json);
    }
    if let Some(result) = crate::ai::rpc::try_dispatch(method, params.clone()).await {
        log::debug!("[rpc:dispatch] routed method={} subsystem=ai", method);
        return result;
    }
    if let Some(result) = crate::openhuman::rpc::try_dispatch(method, params).await {
        log::debug!(
            "[rpc:dispatch] routed method={} subsystem=openhuman",
            method
        );
        return result;
    }

    log::warn!("[rpc:dispatch] unknown_method method={}", method);
    Err(format!("unknown method: {method}"))
}
