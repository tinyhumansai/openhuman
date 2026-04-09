//! Central dispatcher for RPC requests.
//!
//! This module coordinates the routing of incoming requests to either the
//! core subsystem or the OpenHuman domain-specific handlers.

use crate::core::rpc_log;
use crate::core::types::{AppState, InvocationResult};
use serde_json::json;

/// Dispatches an RPC method call to the appropriate subsystem.
///
/// It first attempts to route the request to the core subsystem (e.g., `core.ping`).
/// If not found, it delegates to the `openhuman` domain-specific dispatcher.
///
/// # Arguments
///
/// * `state` - The current application state.
/// * `method` - The name of the RPC method to invoke.
/// * `params` - The parameters for the method call as a JSON value.
///
/// # Returns
///
/// A `Result` containing the JSON-formatted response or an error message.
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

    // Try routing to internal core methods first.
    if let Some(result) = try_core_dispatch(&state, method, params.clone()) {
        log::debug!("[rpc:dispatch] routed method={} subsystem=core", method);
        return result.map(crate::core::types::invocation_to_rpc_json);
    }

    // Delegate to the domain-specific dispatcher.
    if let Some(result) = crate::rpc::try_dispatch(method, params).await {
        log::debug!(
            "[rpc:dispatch] routed method={} subsystem=openhuman",
            method
        );
        return result;
    }

    log::warn!("[rpc:dispatch] unknown_method method={}", method);
    Err(format!("unknown method: {method}"))
}

/// Handles internal core-level RPC methods.
///
/// Currently supports:
/// - `core.ping`: Returns `{ "ok": true }`.
/// - `core.version`: Returns the current core version.
fn try_core_dispatch(
    state: &AppState,
    method: &str,
    _params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "core.ping" => Some(InvocationResult::ok(json!({ "ok": true }))),
        "core.version" => Some(InvocationResult::ok(
            json!({ "version": state.core_version }),
        )),
        _ => None,
    }
}
