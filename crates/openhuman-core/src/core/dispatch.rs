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
    let params = if method == "openhuman.auth_clear_session" {
        // The legacy method was session-only. Keep that contract even though
        // its canonical successor accepts an omitted kind to clear all kinds.
        let mut params = params;
        if let Value::Object(ref mut object) = params {
            object
                .entry("kind".to_string())
                .or_insert_with(|| json!("session"));
        }
        params
    } else {
        params
    };
    let method = resolved;

    // Tier 1: Internal core methods.
    // These are handled directly within the core module and don't require
    // a separate controller registration.
    if let Some(result) = try_core_dispatch(&state, method, params.clone()) {
        log::debug!("[rpc:dispatch] routed method={} subsystem=core", method);
        return result.map(crate::core::types::invocation_to_rpc_json);
    }

    // Tier 2: Registered domain controllers.
    if let Some(result) = try_registry_dispatch(method, params).await {
        log::debug!(
            "[rpc:dispatch] routed method={} subsystem=controller_registry",
            method
        );
        return result;
    }

    // Tier 3: unrecognised method. The JSON-RPC response is unchanged — the
    // caller still receives a method-not-found error. Only the *severity* of
    // how the transport layer records it differs by class (see
    // `jsonrpc::rpc_handler`): known non-actionable misses
    // (`is_known_probe_method`) are debug-only and never reach Sentry
    // (#3567, #3565), while any other unknown method is downgraded to a
    // warn-level capture (recorded for triage, no page) instead of an error
    // event. Log here at debug with the method name so the path stays
    // diagnosable without re-creating the Sentry noise.
    if is_known_probe_method(method) {
        log::debug!(
            "[rpc] unknown_method method={} class=debug_only_known_miss (not reported to Sentry)",
            method
        );
    } else {
        log::debug!(
            "[rpc] unknown_method method={} class=unrecognized (reported to Sentry at warn for triage)",
            method
        );
    }
    Err(format!("{UNKNOWN_METHOD_PREFIX}{method}"))
}

/// Prefix of the error string returned for an unrecognised RPC method. Kept as
/// a shared constant so the emit site (above) and the transport-layer
/// classifier ([`unknown_method_name`]) cannot drift apart.
pub const UNKNOWN_METHOD_PREFIX: &str = "unknown method: ";

/// Known non-actionable unknown method names that are debug-only.
///
/// Most entries are generic external probes that are never real RPC methods
/// and never will be (issue #3567): `rpc.discover` (JSON-RPC service
/// discovery), `list_methods`, liveness `status`, `auth.status`, `config/get`.
/// This also covers retired feature calls from older clients when no safe
/// canonical handler exists (#3565: `openhuman.memory_tree_create_namespace`).
///
///
/// Each miss previously produced recurring Sentry events with zero user
/// impact. The transport layer keeps these debug-only (never captured). The
/// matching health-method *aliases* land separately in `legacy_aliases`
/// (#3566), which depends on this severity change.
const KNOWN_PROBE_METHODS: &[&str] = &[
    "rpc.discover",
    "list_methods",
    "status",
    "auth.status",
    "config/get",
    "openhuman.memory_tree_create_namespace",
];

/// Returns `true` when `method` is a known non-actionable unknown method name
/// from [`KNOWN_PROBE_METHODS`]. Matched against the *resolved* method name
/// (after legacy-alias rewrite), i.e. the name embedded in the
/// [`UNKNOWN_METHOD_PREFIX`] error string.
pub fn is_known_probe_method(method: &str) -> bool {
    KNOWN_PROBE_METHODS.contains(&method)
}

/// Extracts the offending method name from an unknown-method error string, or
/// `None` if `message` is not an unknown-method error. The transport layer uses
/// this to classify the failure for Sentry severity without re-deriving the
/// method from the request (which may differ post legacy-alias rewrite).
pub fn unknown_method_name(message: &str) -> Option<&str> {
    message.strip_prefix(UNKNOWN_METHOD_PREFIX)
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
    params: serde_json::Value,
) -> Option<Result<InvocationResult, String>> {
    match method {
        "core.ping" => Some(InvocationResult::ok(json!({ "ok": true }))),
        "core.version" => Some(InvocationResult::ok(
            json!({ "version": state.core_version }),
        )),
        "core.events_subscribe_token" => Some(handle_events_subscribe_token(params)),
        _ => None,
    }
}

/// Mint a single-shot bind token for the SSE `/events` stream.
///
/// Browser `EventSource` cannot attach an `Authorization` header, so an
/// authenticated holder of the per-process RPC bearer first asks for a
/// short-lived token here (this RPC is gated by the same bearer-token
/// middleware as the rest of `/rpc`) and then opens
/// `/events?client_id=<id>&token=<bind>`. The `/events` handler removes
/// the token from the store on first use, so a leaked URL cannot be
/// replayed by a second subscriber.
fn handle_events_subscribe_token(params: serde_json::Value) -> Result<InvocationResult, String> {
    let obj = params.as_object();
    let client_id = obj
        .and_then(|m| m.get("client_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            log::warn!(
                "[events-bind] reject mint: missing or empty client_id (param_keys={:?})",
                obj.map(|m| m.keys().collect::<Vec<_>>())
            );
            "missing or empty 'client_id' parameter".to_string()
        })?;
    let ttl = obj
        .and_then(|m| m.get("ttl_secs"))
        .and_then(|v| v.as_u64())
        .map(std::time::Duration::from_secs);

    let issued =
        crate::core::event_bind_tokens::issue(client_id.to_string(), ttl).ok_or_else(|| {
            log::warn!(
                "[events-bind] reject mint: store at capacity (client_id_len={} ttl_secs={:?})",
                client_id.len(),
                ttl.map(|d| d.as_secs())
            );
            "events bind-token store at capacity; try again shortly".to_string()
        })?;

    let ttl_remaining_secs = issued
        .valid_until
        .checked_duration_since(std::time::Instant::now())
        .unwrap_or_default()
        .as_secs();

    log::debug!(
        "[events-bind] minted token for client_id_len={} ttl_secs={}",
        client_id.len(),
        ttl_remaining_secs
    );

    InvocationResult::ok(json!({
        "token": issued.token,
        "ttl_secs": ttl_remaining_secs,
    }))
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
#[path = "dispatch_tests.rs"]
mod tests;
