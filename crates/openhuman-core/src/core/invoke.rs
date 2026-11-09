//! In-process controller dispatch: the one entry point every transport calls.
//!
//! The JSON-RPC server in `openhuman-rpc` (HTTP `/rpc` and Socket.IO
//! `rpc:request`), the CLI, the device tunnel and [`CoreRuntime::invoke`](crate::core::runtime::CoreRuntime::invoke)
//! all resolve a method through [`invoke_method`]. It validates params against
//! the controller registry and dispatches. A generic downstream auth error is
//! not enough to sign out: only the request-owning client can publish expiry
//! with the fingerprint of the credential it actually sent.

use serde_json::Value;

use super::session_expiry::{is_session_expired_error, is_unconfirmed_unauthorized_error};
use crate::core::all;
use crate::core::runtime::{CoreContext, DispatchCtx};
use crate::core::types::AppState;

/// Invokes a JSON-RPC method by name.
///
/// This is a high-level wrapper around [`invoke_method_inner`] that adds
/// generic session-error handling. It cannot identify the credential selected
/// by a downstream client, so a hosted 401 remains non-actionable here. The
/// request-owning client publishes expiry with its request-time fingerprint.
/// The offline local credential has no backend session to expire and receives
/// a recoverable hosted-unavailable error instead.
///
/// # Arguments
///
/// * `ctx` - The context the call runs under, built by the transport that
///   accepted it. Handlers receive it as their first argument.
/// * `state` - The application state.
/// * `method` - The name of the method to invoke.
/// * `params` - The JSON parameters for the method.
pub async fn invoke_method(
    ctx: DispatchCtx,
    state: AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    // Generic controller dispatch does not own the hosted client's request
    // credential selection, so its start-time session snapshot is not reliable
    // evidence of which token received a downstream 401.
    let request_credential_fingerprint: Option<String> = None;
    let mut result = invoke_scoped(ctx.clone(), state, method, params).await;

    // Session auto-cleanup: if the OpenHuman auth session is explicitly
    // expired, publish a `SessionExpired` event. The credentials subscriber
    // clears the stored token, flips the scheduler-gate signed-out override
    // so background workers stand down, and (eventually) pushes a sign-out to
    // the UI. Generic downstream/provider 401s must stay recoverable errors;
    // otherwise a scoped integration failure can log the user out.
    if let Err(ref msg) = result {
        let sanitized_reason = tinyinference_core::sanitize::sanitize_api_error(msg);
        if is_session_expired_error(msg) {
            // The credential may have changed while the handler's backend
            // request was in flight. Suppress both the bus event and the RPC
            // error's auth-expired marker so the renderer cannot independently
            // sign out a replacement credential from the response body.
            let current = if let Ok(config) = ctx.load_config().await {
                let token = crate::credentials::session_state::get_session_token(&config)
                    .await
                    .ok()
                    .flatten();
                let local = token
                    .as_deref()
                    .is_some_and(crate::credentials::session_state::is_local_session_token);
                let api_key = crate::credentials::api_key::has_api_key(&config).await;
                Some((token, local, api_key))
            } else {
                None
            };
            let Some((current_token, local, api_key)) = current else {
                log::warn!(
                    "[jsonrpc] session expiry for method='{}' not acted on because current credentials could not be checked",
                    method
                );
                return Err(
                    "Backend request failed while current session state could not be verified"
                        .to_string(),
                );
            };
            if api_key {
                log::info!(
                    "[jsonrpc] session expiry for method='{}' ignored for API-key credentials",
                    method
                );
                result =
                    Err("Backend request failed while using an API-key credential".to_string());
            } else if local {
                log::info!(
                    "[jsonrpc] backend session unavailable for local offline credential method='{}' — leaving local auth intact",
                    method
                );
                // The RPC error is also observed by the renderer's auth
                // classifier. Keep the background hosted probe recoverable.
                result = Err(translate_local_session_error(msg, local)
                    .expect("classified local session error"));
            } else if request_session_failure_is_actionable(
                msg,
                local,
                request_credential_fingerprint.as_deref(),
                current_token.as_deref(),
            ) {
                log::warn!(
                    "[jsonrpc] confirmed session expiry for method='{}' — publishing SessionExpired: {}",
                    method,
                    sanitized_reason
                );
                // `sanitize_api_error` scrubs pasted-through provider replies.
                crate::core::bus::BUS.publish(crate::core::events::DomainEvent::SessionExpired {
                    source: format!("jsonrpc.invoke_method:{method}"),
                    reason: sanitized_reason,
                    credential_fingerprint: request_credential_fingerprint.clone(),
                });
            } else {
                log::info!(
                    "[jsonrpc] session expiry for method='{}' is non-actionable because request credential provenance is unavailable",
                    method
                );
                // Avoid returning an auth-expired marker that would let the
                // renderer sign out a token this request may not have used.
                result = Err(
                    "Backend request failed; credential provenance could not be verified"
                        .to_string(),
                );
            }
        } else if is_unconfirmed_unauthorized_error(msg) {
            log::info!(
                "[jsonrpc] unconfirmed unauthorized error for method='{}' (not session expiry) — leaving session intact: {}",
                method,
                sanitized_reason
            );
        }
    }

    result
}

/// [`invoke_method_inner`] with `ctx` also scoped onto the task, so the schema
/// lookup, parameter validation and fallback dispatch that precede the handler
/// read the context the caller named. Transitional (#7801): goes away with the
/// ambient context once those readers take the context explicitly.
async fn invoke_scoped(
    ctx: DispatchCtx,
    state: AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let core = ctx.core().cloned();
    let call: std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Value, String>> + Send + '_>,
    > = Box::pin(invoke_method_inner(ctx, state, method, params));
    match core {
        Some(core) => CoreContext::scope(core, call).await,
        None => call.await,
    }
}

fn request_session_failure_is_actionable(
    msg: &str,
    credential_is_local: bool,
    request_fingerprint: Option<&str>,
    current_token: Option<&str>,
) -> bool {
    !credential_is_local
        && is_session_expired_error(msg)
        && request_fingerprint
            .zip(current_token)
            .is_some_and(|(fingerprint, token)| {
                crate::credentials::backend_credential::credential_fingerprint_matches(
                    fingerprint,
                    token,
                )
            })
}

fn translate_local_session_error(msg: &str, credential_is_local: bool) -> Option<String> {
    (credential_is_local && is_session_expired_error(msg)).then(|| {
        format!(
            "{}Hosted account data is unavailable in local offline mode",
            crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
        )
    })
}

/// Internal method invocation logic.
///
/// It first attempts to match the method name against the static controller
/// registry (schemas). If a schema is found, it validates the input parameters
/// before execution. If no schema matches, it falls back to the dynamic
/// [`crate::core::dispatch::dispatch`] system.
async fn invoke_method_inner(
    ctx: DispatchCtx,
    state: AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    // Phase 1: Check static controller registry.
    if let Some(schema) = all::schema_for_rpc_method(method) {
        let params_obj = crate::core::params::params_to_object(params.clone())?;
        // Validate inputs against the schema before calling the handler.
        all::validate_params(&schema, &params_obj)?;
        if let Some(result) = all::try_invoke_registered_rpc(ctx.clone(), method, params_obj).await
        {
            return result;
        }
        log::debug!(
            "[jsonrpc] schema matched without registered handler; falling back method={}",
            method
        );
    }

    // Phase 2: Fall back to dynamic dispatch (internal core methods or legacy paths).
    crate::core::dispatch::dispatch(ctx, state, method, params).await
}

/// Returns the default application state.
pub fn default_state() -> AppState {
    AppState {
        core_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

#[cfg(test)]
#[path = "invoke_tests.rs"]
mod tests;
