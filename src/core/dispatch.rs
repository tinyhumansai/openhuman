//! Central dispatcher for RPC requests.
//!
//! This module coordinates the routing of incoming requests to either the
//! core subsystem or the OpenHuman domain-specific handlers.

use crate::core::legacy_aliases::resolve_legacy;
use crate::core::rpc_log;
use crate::core::types::{AppState, InvocationResult};
use serde_json::{json, Map, Value};

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
    let method = if let Some(canonical) = normalize_legacy_method(method) {
        log::debug!(
            "[rpc] legacy method '{}' rewritten to '{}'",
            method,
            canonical
        );
        canonical
    } else {
        method
    };

    log::trace!(
        "[rpc:dispatch] enter method={} params={}",
        method,
        rpc_log::redact_params_for_log(&params)
    );

    // Tier 0: Rewrite legacy method names to their canonical form before
    // any subsystem lookup. Symmetric with the frontend's
    // `normalizeRpcMethod` (`app/src/services/rpcMethods.ts`): the
    // frontend rewrites outgoing names for clients that just updated, the
    // core rewrites incoming names for clients that haven't yet. See
    // `crate::core::legacy_aliases` for the shared table.
    let resolved = resolve_legacy(method);
    if resolved != method {
        // Per-rewrite log at debug to keep the dispatcher hot path quiet
        // at scale (per graycyrus review on PR #1544). Aggregate
        // visibility belongs in the observability layer, not here.
        log::debug!(
            "[rpc-legacy-alias] rewrite method={} -> canonical={}",
            method,
            resolved
        );
    }
    let method = resolved;

    // Tier 1: Internal core methods.
    // These are handled directly within the core module and don't require
    // a separate controller registration.
    if let Some(result) = try_core_dispatch(&state, method, params.clone()) {
        log::debug!("[rpc:dispatch] routed method={} subsystem=core", method);
        return result.map(crate::core::types::invocation_to_rpc_json);
    }

    // Tier 2: Registered domain controllers.
    if let Some(result) = try_registry_dispatch(method, params.clone()).await {
        log::debug!(
            "[rpc:dispatch] routed method={} subsystem=controller_registry",
            method
        );
        return result;
    }

    // Tier 3: Legacy domain-specific dispatcher.
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

/// Normalizes legacy un-namespaced method names to their canonical equivalents.
///
/// This provides defense-in-depth against stale or unbalanced callers that may
/// still be using old method names.
///
/// Source of truth: `app/src/services/rpcMethods.ts` (`LEGACY_METHOD_ALIASES`)
fn normalize_legacy_method(method: &str) -> Option<&'static str> {
    match method {
        "openhuman.get_analytics_settings" => Some("openhuman.config_get_analytics_settings"),
        "openhuman.get_composio_trigger_settings" => {
            Some("openhuman.config_get_composio_trigger_settings")
        }
        "openhuman.get_config" => Some("openhuman.config_get"),
        "openhuman.get_runtime_flags" => Some("openhuman.config_get_runtime_flags"),
        "openhuman.ping" => Some("core.ping"),
        "openhuman.set_browser_allow_all" => Some("openhuman.config_set_browser_allow_all"),
        "openhuman.update_analytics_settings" => Some("openhuman.config_update_analytics_settings"),
        "openhuman.update_browser_settings" => Some("openhuman.config_update_browser_settings"),
        "openhuman.update_composio_trigger_settings" => {
            Some("openhuman.config_update_composio_trigger_settings")
        }
        "openhuman.update_local_ai_settings" => Some("openhuman.config_update_local_ai_settings"),
        "openhuman.update_memory_settings" => Some("openhuman.config_update_memory_settings"),
        "openhuman.update_model_settings" => Some("openhuman.config_update_model_settings"),
        "openhuman.update_runtime_settings" => Some("openhuman.config_update_runtime_settings"),
        "openhuman.update_screen_intelligence_settings" => {
            Some("openhuman.config_update_screen_intelligence_settings")
        }
        "openhuman.workspace_onboarding_flag_exists" => {
            Some("openhuman.config_workspace_onboarding_flag_exists")
        }
        "openhuman.workspace_onboarding_flag_set" => {
            Some("openhuman.config_workspace_onboarding_flag_set")
        }
        _ => None,
    }
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

async fn try_registry_dispatch(
    method: &str,
    params: Value,
) -> Option<Result<serde_json::Value, String>> {
    let schema = crate::core::all::schema_for_rpc_method(method)?;
    let params_obj = match params_to_object(params) {
        Ok(params_obj) => params_obj,
        Err(err) => return Some(Err(err)),
    };
    if let Err(err) = crate::core::all::validate_params(&schema, &params_obj) {
        return Some(Err(err));
    }
    crate::core::all::try_invoke_registered_rpc(method, params_obj).await
}

fn params_to_object(params: Value) -> Result<Map<String, Value>, String> {
    match params {
        Value::Object(map) => Ok(map),
        Value::Null => Ok(Map::new()),
        other => Err(format!(
            "invalid params: expected object or null, got {}",
            type_name(&other)
        )),
    }
}

fn type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
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
    async fn dispatch_rewrites_legacy_alias_before_lookup() {
        // `openhuman.ping` is a legacy alias for `core.ping` in the shared
        // alias table. Going through the dispatcher must rewrite it and
        // route successfully to Tier 1 instead of falling through to the
        // unknown-method error path.
        let out = dispatch(test_state(), "openhuman.ping", json!({}))
            .await
            .expect("legacy alias openhuman.ping must resolve to core.ping");
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

    #[test]
    fn test_normalize_legacy_method_all_aliases() {
        let cases = vec![
            (
                "openhuman.get_analytics_settings",
                "openhuman.config_get_analytics_settings",
            ),
            (
                "openhuman.get_composio_trigger_settings",
                "openhuman.config_get_composio_trigger_settings",
            ),
            ("openhuman.get_config", "openhuman.config_get"),
            (
                "openhuman.get_runtime_flags",
                "openhuman.config_get_runtime_flags",
            ),
            ("openhuman.ping", "core.ping"),
            (
                "openhuman.set_browser_allow_all",
                "openhuman.config_set_browser_allow_all",
            ),
            (
                "openhuman.update_analytics_settings",
                "openhuman.config_update_analytics_settings",
            ),
            (
                "openhuman.update_browser_settings",
                "openhuman.config_update_browser_settings",
            ),
            (
                "openhuman.update_composio_trigger_settings",
                "openhuman.config_update_composio_trigger_settings",
            ),
            (
                "openhuman.update_local_ai_settings",
                "openhuman.config_update_local_ai_settings",
            ),
            (
                "openhuman.update_memory_settings",
                "openhuman.config_update_memory_settings",
            ),
            (
                "openhuman.update_model_settings",
                "openhuman.config_update_model_settings",
            ),
            (
                "openhuman.update_runtime_settings",
                "openhuman.config_update_runtime_settings",
            ),
            (
                "openhuman.update_screen_intelligence_settings",
                "openhuman.config_update_screen_intelligence_settings",
            ),
            (
                "openhuman.workspace_onboarding_flag_exists",
                "openhuman.config_workspace_onboarding_flag_exists",
            ),
            (
                "openhuman.workspace_onboarding_flag_set",
                "openhuman.config_workspace_onboarding_flag_set",
            ),
        ];

        for (legacy, canonical) in cases {
            assert_eq!(
                normalize_legacy_method(legacy),
                Some(canonical),
                "Legacy method {} should normalize to {}",
                legacy,
                canonical
            );
        }
    }

    #[test]
    fn test_normalize_legacy_method_none_for_unknown_or_canonical() {
        assert!(normalize_legacy_method("core.ping").is_none());
        assert!(normalize_legacy_method("openhuman.config_get").is_none());
        assert!(normalize_legacy_method("unknown.method").is_none());
    }

    #[tokio::test]
    async fn dispatch_legacy_ping_rewrites_and_succeeds() {
        let out = dispatch(test_state(), "openhuman.ping", json!({}))
            .await
            .expect("openhuman.ping should be rewritten to core.ping and succeed");
        assert_eq!(out, json!({ "ok": true }));
    }

    #[tokio::test]
    async fn dispatch_legacy_alias_routes_to_registry() {
        // openhuman.get_analytics_settings should rewrite to openhuman.config_get_analytics_settings.
        // This is a read-only call and should succeed if the registry is wired up.
        let out = dispatch(test_state(), "openhuman.get_analytics_settings", json!({}))
            .await
            .expect("openhuman.get_analytics_settings should be rewritten and succeed");

        // The registry-wrapped payload has a "result" field.
        assert!(
            out.get("enabled").is_some() || out.get("result").is_some(),
            "Payload should have 'enabled' or 'result', got: {}",
            out
        );
    }
}
