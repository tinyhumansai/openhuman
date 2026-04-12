//! Central dispatcher for RPC requests.
//!
//! This module coordinates the routing of incoming requests to either the
//! core subsystem or the OpenHuman domain-specific handlers.

use crate::core::rpc_log;
use crate::core::types::{AppState, InvocationResult};
use serde_json::json;

/// Dispatches an RPC method call to the appropriate subsystem.
///
/// This is the primary entry point for all RPC calls. It uses a tiered routing
/// strategy:
/// 1. **Core Subsystem**: Checks for internal methods like `core.ping` or `core.version`.
/// 2. **Domain-Specific Handlers**: Delegates to the `openhuman` domain dispatcher
///    which handles all registered controllers (memory, skills, etc.).
///
/// # Arguments
///
/// * `state` - The current application state (e.g., core version).
/// * `method` - The name of the RPC method to invoke (e.g., `core.ping`).
/// * `params` - The parameters for the method call as a JSON value.
///
/// # Returns
///
/// A `Result` containing the JSON-formatted response or an error message if
/// the method is unknown or invocation fails.
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

    // Tier 1: Internal core methods.
    // These are handled directly within the core module and don't require
    // a separate controller registration.
    if let Some(result) = try_core_dispatch(&state, method, params.clone()) {
        log::debug!("[rpc:dispatch] routed method={} subsystem=core", method);
        return result.map(crate::core::types::invocation_to_rpc_json);
    }

    // Tier 2: Domain-specific dispatcher.
    // This routes to controllers registered in src/core/all.rs and src/rpc/mod.rs.
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
/// These methods provide basic information about the server and its version.
///
/// Currently supported methods:
/// - `core.ping`: A simple liveness check. Returns `{ "ok": true }`.
/// - `core.version`: Returns the version of the running core binary.
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
