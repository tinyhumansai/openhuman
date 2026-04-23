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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_state() -> AppState {
        AppState {
            core_version: "9.9.9-test".to_string(),
        }
    }

    #[tokio::test]
    async fn dispatch_core_ping_returns_ok_true() {
        let out = dispatch(test_state(), "core.ping", json!({}))
            .await
            .expect("core.ping should succeed");
        assert_eq!(out, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn dispatch_core_version_returns_state_version() {
        let out = dispatch(test_state(), "core.version", json!({}))
            .await
            .expect("core.version should succeed");
        assert_eq!(out, json!({ "version": "9.9.9-test" }));
    }

    #[tokio::test]
    async fn dispatch_core_ignores_params() {
        // Params must be tolerated even when the method takes none.
        let out = dispatch(test_state(), "core.ping", json!({ "extra": 1 }))
            .await
            .expect("core.ping should ignore extra params");
        assert_eq!(out, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_error() {
        let err = dispatch(test_state(), "does.not.exist", json!({}))
            .await
            .expect_err("unknown methods must error");
        assert!(err.contains("unknown method"));
        assert!(err.contains("does.not.exist"));
    }

    #[tokio::test]
    async fn dispatch_empty_method_returns_unknown_method_error() {
        let err = dispatch(test_state(), "", json!({}))
            .await
            .expect_err("empty method must error");
        assert!(err.contains("unknown method"));
    }

    #[tokio::test]
    async fn dispatch_delegates_to_tier2_for_domain_method() {
        // Tier 2 dispatcher handles `openhuman.security_policy_info`, so
        // it must succeed and return a policy object.
        let out = dispatch(test_state(), "openhuman.security_policy_info", json!({}))
            .await
            .expect("security_policy_info should route via tier 2");
        // With logs present, payload is wrapped as { result, logs }.
        assert!(out.get("result").is_some() || out.get("autonomy").is_some());
    }

    #[test]
    fn try_core_dispatch_returns_none_for_non_core_namespace() {
        let state = test_state();
        assert!(try_core_dispatch(&state, "openhuman.memory_list_namespaces", json!({})).is_none());
        assert!(try_core_dispatch(&state, "corez.ping", json!({})).is_none());
    }

    #[test]
    fn try_core_dispatch_matches_exact_ping_and_version() {
        let state = test_state();
        assert!(try_core_dispatch(&state, "core.ping", json!({})).is_some());
        assert!(try_core_dispatch(&state, "core.version", json!({})).is_some());
        // Prefix match alone must not count.
        assert!(try_core_dispatch(&state, "core.pingz", json!({})).is_none());
        assert!(try_core_dispatch(&state, "core", json!({})).is_none());
    }

    #[test]
    fn try_core_dispatch_version_reflects_appstate() {
        let state = AppState {
            core_version: "0.0.0-abc".into(),
        };
        let result = try_core_dispatch(&state, "core.version", json!({}))
            .expect("core.version must be routed")
            .expect("core.version must produce InvocationResult");
        assert_eq!(result.value, json!({ "version": "0.0.0-abc" }));
        assert!(result.logs.is_empty());
    }
}
