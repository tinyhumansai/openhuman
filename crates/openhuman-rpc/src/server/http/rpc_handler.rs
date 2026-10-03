//! `POST /rpc`: the JSON-RPC 2.0 endpoint.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use openhuman_core::core::invoke::invoke_method;
use openhuman_core::core::types::AppState;
use openhuman_core::core::StructuredRpcError;

use super::super::classify::{classify_failure, FailureDisposition, USAGE_BACKOFF_CLIENT_MESSAGE};
use crate::{RpcError, RpcFailure, RpcRequest, RpcSuccess, JSONRPC_VERSION, SERVER_ERROR_CODE};

/// Axum handler for JSON-RPC POST requests.
///
/// This function:
/// 1. Receives a JSON-RPC request body.
/// 2. Extracts the method name and parameters.
/// 3. Invokes the corresponding handler via [`invoke_method`].
/// 4. Wraps the result or error in a JSON-RPC 2.0 compliant response.
///
/// Failures are reported according to [`classify_failure`]; see
/// [`FailureDisposition`] for why each class is or is not a Sentry event.
///
/// # Arguments
///
/// * `state` - The application state, injected by Axum.
/// * `req` - The parsed [`RpcRequest`].
pub async fn rpc_handler(State(state): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    let id = req.id.clone();
    let method = req.method.clone();
    let started = std::time::Instant::now();
    let result = invoke_method(state, method.as_str(), req.params).await;
    let ms = started.elapsed().as_millis();

    match result {
        Ok(value) => {
            tracing::info!("[rpc] {} -> ok ({}ms)", method, ms);
            (
                StatusCode::OK,
                Json(RpcSuccess {
                    jsonrpc: JSONRPC_VERSION,
                    id,
                    result: value,
                }),
            )
                .into_response()
        }
        Err(raw_message) => {
            // Decode the controller-emitted structured envelope (if any)
            // here at the transport boundary. Domains opt in by emitting a
            // `StructuredRpcError` from their handlers — this layer never
            // branches on the RPC method name to recover error semantics.
            let structured = StructuredRpcError::decode(&raw_message);
            let (mut display_message, error_data, expected_user_state) = match structured {
                Some(envelope) => (
                    envelope.message,
                    envelope.data,
                    envelope.expected_user_state,
                ),
                None => (raw_message, None, false),
            };

            match classify_failure(&display_message, expected_user_state) {
                FailureDisposition::ExpectedUserState => {
                    tracing::info!(
                        method = %method,
                        "[rpc] expected-user-state error — skipping Sentry: {}",
                        display_message
                    );
                }
                FailureDisposition::WalletNotConfigured => {
                    tracing::info!(
                        method = %method,
                        "[rpc] wallet-not-configured (expected user-state) — skipping Sentry"
                    );
                }
                FailureDisposition::ParamValidation => {
                    tracing::info!(
                        method = %method,
                        elapsed_ms = ms as u64,
                        "[rpc] param-validation error (message redacted; skip-report)"
                    );
                }
                FailureDisposition::SessionExpired => {
                    tracing::info!("[rpc] {} -> err ({}ms): {}", method, ms, display_message);
                }
                FailureDisposition::UsageProbeBackoff => {
                    tracing::debug!(
                        method = %method,
                        elapsed_ms = ms as u64,
                        "[rpc] usage-probe failure-backoff repeat — not reporting to Sentry"
                    );
                    display_message = USAGE_BACKOFF_CLIENT_MESSAGE.to_string();
                }
                FailureDisposition::TransientDownstream => {
                    let redacted =
                        tinyinference_core::sanitize::sanitize_api_error(&display_message);
                    tracing::warn!(
                        method = %method,
                        elapsed_ms = ms as u64,
                        error = %redacted,
                        "[rpc] transient downstream failure — not reporting to Sentry (message redacted)"
                    );
                }
                FailureDisposition::UnknownMethod { probe: true } => {
                    tracing::debug!(
                        method = %method,
                        elapsed_ms = ms as u64,
                        "[rpc] unknown probe/legacy method (allow-listed) — debug only, not reporting to Sentry"
                    );
                }
                FailureDisposition::UnknownMethod { probe: false } => {
                    openhuman_core::core::observability::report_warning_message(
                        display_message.as_str(),
                        "rpc",
                        "invoke_method",
                        &[("method", method.as_str()), ("elapsed_ms", &ms.to_string())],
                    );
                }
                FailureDisposition::Unexpected => {
                    openhuman_core::core::observability::report_error_or_expected(
                        display_message.as_str(),
                        "rpc",
                        "invoke_method",
                        &[("method", method.as_str()), ("elapsed_ms", &ms.to_string())],
                    );
                }
            }
            (
                StatusCode::OK,
                Json(RpcFailure {
                    jsonrpc: JSONRPC_VERSION,
                    id,
                    error: RpcError {
                        code: SERVER_ERROR_CODE,
                        message: display_message,
                        data: error_data,
                    },
                }),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
#[path = "rpc_handler_tests.rs"]
mod tests;
