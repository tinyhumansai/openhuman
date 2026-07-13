//! JSON-RPC 2.0 server implementation for OpenHuman.
//!
//! This module provides:
//! - An Axum-based HTTP server for handling JSON-RPC requests.
//! - Method dispatching to registered controllers.
//! - SSE (Server-Sent Events) for real-time event streaming.
//! - Helper routes for health checks, schema discovery, and Telegram authentication.

use std::sync::Arc;

use axum::extract::{DefaultBodyLimit, Query, State, WebSocketUpgrade};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{extract::Request, Json, Router};
use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio_stream::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::core::all;
use crate::core::types::{AppState, RpcError, RpcFailure, RpcRequest, RpcSuccess};
use crate::rpc::StructuredRpcError;

/// Axum handler for JSON-RPC POST requests.
///
/// This function:
/// 1. Receives a JSON-RPC request body.
/// 2. Extracts the method name and parameters.
/// 3. Invokes the corresponding handler via [`invoke_method`].
/// 4. Wraps the result or error in a JSON-RPC 2.0 compliant response.
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
                    jsonrpc: "2.0",
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

            // Session-expired bubbles up as an "error" but is an expected
            // boundary condition (auth handler clears the local token and the
            // UI re-auths). Don't spam Sentry with it.
            //
            // Param-validation failures ("unknown param 'x' for ns.fn",
            // "missing required param 'x'", "invalid params: …") are also
            // pure boundary mismatches: either the caller is a frontend on a
            // different release than the running core (OPENHUMAN-TAURI-20:
            // v0.53.22 UI shipped `api_key` before the matching schema input
            // landed in #1467) or it is straight client-bug input. Sentry
            // cannot help — we can neither retro-fix already-shipped
            // installs nor learn anything from the noise — so log at info
            // and skip the report.
            //
            // Logging asymmetry between the two skip paths is intentional:
            // session-expired messages are a small set of fixed strings
            // (no caller-supplied content), so the full text is safe to
            // log. Param-validation messages embed caller-supplied param
            // names and, for the `invalid params: …` shape, can carry
            // deserialized values — log structurally with redacted body
            // to keep PII out of the sink while preserving the method
            // for grep / correlation.
            //
            // Domains that surface their own expected-user-state errors
            // (stale thread refs, etc.) set the `expected_user_state` flag
            // on their structured envelope and skip Sentry here uniformly.
            if expected_user_state {
                tracing::info!(
                    method = %method,
                    "[rpc] expected-user-state error — skipping Sentry: {}",
                    display_message
                );
            } else if is_wallet_not_configured_error(&display_message) {
                // A `tinyplace_*` RPC needs a wallet-derived signer but the user
                // has not set one up. Expected user-state (the UI shows a
                // "set up wallet" prompt), not an internal failure — skip Sentry
                // here so the message is left untouched for direct (agent-tool)
                // callers. See `is_wallet_not_configured_error`.
                tracing::info!(
                    method = %method,
                    "[rpc] wallet-not-configured (expected user-state) — skipping Sentry"
                );
            } else if is_param_validation_error(&display_message) {
                tracing::info!(
                    method = %method,
                    elapsed_ms = ms as u64,
                    "[rpc] param-validation error (message redacted; skip-report)"
                );
            } else if is_session_expired_error(&display_message) {
                tracing::info!("[rpc] {} -> err ({}ms): {}", method, ms, display_message);
            } else if crate::core::observability::is_suppressed_usage_probe_backoff(
                &display_message,
            ) {
                // A `/teams/me/usage` probe that the failure-backoff in
                // `team::ops` short-circuited within its window — i.e. an
                // already-reported repeat. The FIRST failure of the streak
                // already hit the backend and reported here normally; demoting
                // the repeats is exactly the flood control GH #4153 asks for
                // (backpressure, not silent drop). Debug-only, never Sentry.
                tracing::debug!(
                    method = %method,
                    elapsed_ms = ms as u64,
                    "[rpc] usage-probe failure-backoff repeat — not reporting to Sentry"
                );
                // Keep the internal demotion marker strictly out-of-band: it
                // exists only to drive the Sentry-demotion decision above and
                // must never reach the RPC client as the error message. Replace
                // it with a clean, user-presentable string before the response
                // is built below (CodeRabbit on #4153 — don't leak the sentinel).
                display_message =
                    "Usage temporarily unavailable — the last fetch failed and is backing off; \
                     it will refresh shortly."
                        .to_string();
            } else if crate::core::observability::is_transient_message_failure(&display_message) {
                // Downstream call (backend_api / integrations / provider) already
                // demoted the underlying transient failure to a warn. The error
                // string still propagates up to here; re-reporting at error level
                // would re-create the very Sentry noise the lower-layer demote
                // was meant to avoid (#8Z, #93, #8W, #96).
                //
                // Redact before logging — `display_message` is upstream-derived
                // (backend / provider response) and can carry URL fragments,
                // query params, or pasted-through provider error text that
                // includes tokens. `sanitize_api_error` runs the same scrub
                // used in the SessionExpired publish path below.
                let redacted = crate::openhuman::inference::provider::ops::sanitize_api_error(
                    &display_message,
                );
                tracing::warn!(
                    method = %method,
                    elapsed_ms = ms as u64,
                    error = %redacted,
                    "[rpc] transient downstream failure — not reporting to Sentry (message redacted)"
                );
            } else if let Some(unknown_method) =
                crate::core::dispatch::unknown_method_name(&display_message)
            {
                // An unrecognised RPC method is a transport-boundary mismatch
                // (infra probe traffic, or a client on a different release than
                // the running core), not an actionable core defect (#3567).
                // Known external probes never become real methods, so they are
                // debug-only and never reach Sentry; any other unknown method
                // is still recorded for triage but at warn severity (captured,
                // no page) rather than an error event. Either way the JSON-RPC
                // method-not-found response to the caller below is unchanged.
                if crate::core::dispatch::is_known_probe_method(unknown_method) {
                    tracing::debug!(
                        method = %method,
                        elapsed_ms = ms as u64,
                        "[rpc] unknown probe/legacy method (allow-listed) — debug only, not reporting to Sentry"
                    );
                } else {
                    crate::core::observability::report_warning_message(
                        display_message.as_str(),
                        "rpc",
                        "invoke_method",
                        &[("method", method.as_str()), ("elapsed_ms", &ms.to_string())],
                    );
                }
            } else {
                crate::core::observability::report_error_or_expected(
                    display_message.as_str(),
                    "rpc",
                    "invoke_method",
                    &[("method", method.as_str()), ("elapsed_ms", &ms.to_string())],
                );
            }
            (
                StatusCode::OK,
                Json(RpcFailure {
                    jsonrpc: "2.0",
                    id,
                    error: RpcError {
                        code: -32000,
                        message: display_message,
                        data: error_data,
                    },
                }),
            )
                .into_response()
        }
    }
}

/// Invokes a JSON-RPC method by name.
///
/// This is a high-level wrapper around [`invoke_method_inner`] that adds
/// automatic session management logic. If a call fails with a confirmed
/// OpenHuman session-expired error, it will automatically clear the local
/// session.
///
/// # Arguments
///
/// * `state` - The application state.
/// * `method` - The name of the method to invoke.
/// * `params` - The JSON parameters for the method.
pub async fn invoke_method(state: AppState, method: &str, params: Value) -> Result<Value, String> {
    let result = invoke_method_inner(state, method, params).await;

    // Session auto-cleanup: if the OpenHuman auth session is explicitly
    // expired, publish a `SessionExpired` event. The credentials subscriber
    // clears the stored token, flips the scheduler-gate signed-out override
    // so background workers stand down, and (eventually) pushes a sign-out to
    // the UI. Generic downstream/provider 401s must stay recoverable errors;
    // otherwise a scoped integration failure can log the user out.
    if let Err(ref msg) = result {
        let sanitized_reason = crate::openhuman::inference::provider::ops::sanitize_api_error(msg);
        if is_session_expired_error(msg) {
            log::warn!(
                "[jsonrpc] confirmed session expiry for method='{}' — publishing SessionExpired: {}",
                method,
                sanitized_reason
            );
            // pasted-through provider replies. `sanitize_api_error` runs
            // `scrub_secret_patterns` and truncates.
            //
            // Local-session protection is handled by `SessionExpiredSubscriber`
            // in `src/openhuman/credentials/bus.rs` — it checks `is_local_session_token`
            // after config load and short-circuits teardown with
            // `scheduler_gate::set_signed_out(false)`. Duplicating that check
            // here would pull a domain concern into the transport layer and would
            // add an extra config-load round-trip on every 401.
            crate::core::event_bus::publish_global(
                crate::core::event_bus::DomainEvent::SessionExpired {
                    source: format!("jsonrpc.invoke_method:{method}"),
                    reason: sanitized_reason,
                },
            );
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

/// Helper to determine if an error message indicates an expired or invalid
/// OpenHuman backend session.
///
/// **Narrower than the previous implementation** (fixed in issue #2286):
///
/// The old predicate matched ANY `"401 + unauthorized"` pattern, which caused
/// downstream provider 401s (Discord bot token failures, BYO-key OpenAI /
/// Anthropic failures, Composio direct-mode errors) to clear the user's session
/// and log them out. The fix distinguishes between:
///
/// - **OpenHuman backend 401s** (`authed_json` in `src/api/rest.rs`): formatted
///   as `"{METHOD} /path failed (401 Unauthorized): {body}"`, e.g.
///   `"GET /teams failed (401 Unauthorized): {"success":false}"`. These always
///   start with an HTTP method verb followed by a space and a forward slash.
/// - **Provider / downstream 401s** (`api_error` in
///   `src/openhuman/inference/provider/ops.rs`): formatted as
///   `"{ProviderName} API error (401 Unauthorized): {body}"` or
///   `"Discord API error: ... (401): Unauthorized"`. These start with a
///   provider name, NOT an HTTP method verb.
///
/// **What still triggers session expiry:**
/// - `"Session expired"` — explicit body text from the OpenHuman backend.
/// - `"no backend session token"` — pre-flight guard; auth profile is missing.
/// - `"session jwt required"` — local guard; JWT already cleared by a prior 401.
/// - `"SESSION_EXPIRED"` — scheduler-gate sentinel (exact case).
/// - HTTP-method-prefixed 401s (`GET /`, `POST /`, etc.) — backend path format.
///
/// **What no longer triggers session expiry (fixed in #2286):**
/// - Provider-prefixed 401s (`"Discord API error: ..."`, `"OpenAI API error ..."`)
/// - `"invalid token"` — too broad; also matches Discord / OAuth provider tokens.
///
/// Note: for inference-path OpenHuman backend 401s, `api_error` (in
/// `inference/provider/ops.rs` lines 479–497) ALREADY publishes `SessionExpired`
/// directly, so there is no regression if this predicate misses them — the
/// subscriber is idempotent and a harmless double-publish would still be correct.
fn is_session_expired_error(msg: &str) -> bool {
    // Explicit session-expired markers from the OpenHuman backend / local
    // guards — delegated to the shared observability classifier so both the
    // Sentry expected-error pipeline and the JSON-RPC publish boundary stay
    // in lock-step.
    if crate::core::observability::is_session_expired_message(msg) {
        return true;
    }
    // OpenHuman backend path 401s via `authed_json`:
    // format is "{METHOD} /path failed (401 Unauthorized): {body}"
    // The HTTP-method prefix distinguishes these from provider-prefixed errors.
    // HEAD and OPTIONS are intentionally excluded — `authed_json` only issues
    // the five listed verbs (GET/POST/PUT/DELETE/PATCH) for REST JSON endpoints.
    let lower = msg.to_ascii_lowercase();
    if (lower.contains("401") && lower.contains("unauthorized"))
        && (msg.starts_with("GET /")
            || msg.starts_with("POST /")
            || msg.starts_with("PUT /")
            || msg.starts_with("DELETE /")
            || msg.starts_with("PATCH /"))
    {
        return true;
    }
    false
}

/// Detect auth-looking failures that are not specific enough to clear the
/// OpenHuman session. This is only for diagnostics; it must not feed the
/// `SessionExpired` publish path.
///
/// Matches a generic `401 Unauthorized` OR a bare `"invalid token"` string,
/// either of which can come from BYO-key providers, Composio, channels, or
/// other scoped downstream calls. Used exclusively for diagnostic logging
/// at the `invoke_method` call site so provider auth failures are visible
/// in the logs without being misclassified as session expiry.
fn is_unconfirmed_unauthorized_error(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    (lower.contains("401") && lower.contains("unauthorized")) || lower.contains("invalid token")
}

/// Returns `true` when the error message comes from JSON-RPC params validation
/// rather than the underlying handler.
///
/// Three shapes, all emitted before the handler ever runs:
///   * `"unknown param '<key>' for <ns>.<fn>"`       — `all::validate_params` (extra field)
///   * `"missing required param '<key>': <comment>"` — `all::validate_params` (omitted required field)
///   * `"invalid params: expected object or null, got <type>"` — `params_to_object` (wrong params shape)
///
/// These only fire when caller and server schemas drift at the transport layer
/// — either a frontend on a different release than the running core, or a buggy
/// external client. Reporting them to Sentry produces unactionable noise (we
/// cannot patch an already-shipped install, and the message itself already
/// names the bad field).
///
/// Note: domain-level validation errors (e.g. type/format checks emitted *inside*
/// a controller's `rpc.rs` handler such as `"param 'x' must be a UUID"`) are
/// intentionally *not* matched here — only the three shapes emitted by the
/// transport-layer validators before the handler runs. Longer-term a typed
/// `RpcError::ParamValidation` variant would remove the string-matching
/// brittleness; the unit tests in `jsonrpc_tests.rs` lock the exact prefixes
/// against the emit sites in `all::validate_params` and `params_to_object`.
///
/// `starts_with` (not `.contains()`) is deliberate: validator errors are always
/// emitted as the full message body, so an anchored match avoids false positives
/// from upstream handler text that happens to mention `"unknown param"`. The
/// session-expired predicate uses `.contains()` because session-expired markers
/// can appear mid-message — flip these to match and the test
/// `is_param_validation_error_does_not_match_unrelated_errors` will break.
fn is_param_validation_error(msg: &str) -> bool {
    msg.starts_with("unknown param '")
        || msg.starts_with("missing required param '")
        || msg.starts_with("invalid params: ")
}

/// Returns `true` when the error is the wallet's "not configured yet" message.
///
/// Several `tinyplace_*` RPCs derive a signer seed from the wallet before they
/// can run (the feed, signal/messaging, etc. — backend `GraphQLAuth::Agent`
/// requires a signer). For a user who has not set up a wallet, the wallet layer
/// returns [`crate::openhuman::wallet::WALLET_NOT_CONFIGURED_MESSAGE`]. That is
/// an expected user-state, not an internal failure: the UI already renders a
/// "set up wallet" prompt, and there is no local lever to make the call succeed
/// until the user creates a wallet. Classifying it here — at the single Sentry
/// boundary — keeps it out of Sentry for *every* path that surfaces it (the
/// shared client builder and the direct `signal_store` seed call alike) without
/// the controllers returning a structured envelope, which would leak the raw
/// sentinel string to agent tools that call those handlers directly.
///
/// Matched against the shared wallet constant (exact equality) so a wording
/// change in the wallet layer fails the coupling test in `jsonrpc_tests.rs`
/// rather than silently letting the noise back into Sentry.
fn is_wallet_not_configured_error(msg: &str) -> bool {
    msg == crate::openhuman::wallet::WALLET_NOT_CONFIGURED_MESSAGE
}

/// Internal method invocation logic.
///
/// It first attempts to match the method name against the static controller
/// registry (schemas). If a schema is found, it validates the input parameters
/// before execution. If no schema matches, it falls back to the dynamic
/// [`crate::core::dispatch::dispatch`] system.
async fn invoke_method_inner(
    state: AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    // Phase 1: Check static controller registry.
    if let Some(schema) = all::schema_for_rpc_method(method) {
        let params_obj = params_to_object(params.clone())?;
        // Validate inputs against the schema before calling the handler.
        all::validate_params(&schema, &params_obj)?;
        if let Some(result) = all::try_invoke_registered_rpc(method, params_obj).await {
            return result;
        }
        log::debug!(
            "[jsonrpc] schema matched without registered handler; falling back method={}",
            method
        );
    }

    // Phase 2: Fall back to dynamic dispatch (internal core methods or legacy paths).
    crate::core::dispatch::dispatch(state, method, params).await
}

/// Converts JSON parameters into a map, ensuring they are in object format.
///
/// JSON-RPC allows parameters to be an Object, an Array, or Null. This implementation
/// primarily supports Object parameters for named-argument style calls.
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

/// Returns a human-readable string representation of a JSON value's type.
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

/// Parses a JSON string into a `Value`.
pub fn parse_json_params(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid JSON params: {e}"))
}

/// Returns the default application state.
pub fn default_state() -> AppState {
    AppState {
        core_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

// --- HTTP server (Axum) ----------------------------------------------------

/// Query parameters for the Telegram authentication callback.
#[derive(Debug, serde::Deserialize)]
struct TelegramAuthQuery {
    /// The one-time login token received from the Telegram bot.
    token: Option<String>,
}

/// Query parameters for the generic desktop auth callback.
#[derive(Debug, serde::Deserialize)]
struct DesktopAuthQuery {
    /// One-time login token consumed through the backend.
    token: Option<String>,
    /// Deprecated backend marker for direct session JWT callbacks.
    key: Option<String>,
}

/// Returns the HTML for a successful connection page.
fn success_html(message: &str) -> String {
    let escaped_message = escape_html(message);
    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>OpenHuman &#8212; Connected</title>
    <style>
        * { margin: 0; padding: 0; box-sizing: border-box; }
        body { font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; display: flex; align-items: center; justify-content: center; min-height: 100vh; }
        .card { background: #1e293b; border-radius: 16px; padding: 48px; text-align: center; max-width: 420px; box-shadow: 0 20px 25px -5px rgba(0,0,0,0.3); }
        .icon { font-size: 48px; margin-bottom: 16px; }
        h1 { font-size: 24px; margin-bottom: 12px; color: #f8fafc; }
        p { font-size: 16px; color: #94a3b8; line-height: 1.6; }
    </style>
</head>
<body>
    <div class="card">
        <div class="icon">&#10004;</div>
        <h1>Connected!</h1>
        <p>__MESSAGE__</p>
    </div>
</body>
</html>"#
    .replace("__MESSAGE__", &escaped_message)
}

/// Simple HTML escaping for error messages.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

/// Returns the HTML for an error page.
fn error_html(message: &str) -> String {
    let escaped_message = escape_html(message);
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <meta name="viewport" content="width=device-width, initial-scale=1">
    <title>OpenHuman &#8212; Error</title>
    <style>
        * {{ margin: 0; padding: 0; box-sizing: border-box; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif; background: #0f172a; color: #e2e8f0; display: flex; align-items: center; justify-content: center; min-height: 100vh; }}
        .card {{ background: #1e293b; border-radius: 16px; padding: 48px; text-align: center; max-width: 420px; box-shadow: 0 20px 25px -5px rgba(0,0,0,0.3); }}
        .icon {{ font-size: 48px; margin-bottom: 16px; }}
        h1 {{ font-size: 24px; margin-bottom: 12px; color: #f8fafc; }}
        p {{ font-size: 16px; color: #94a3b8; line-height: 1.6; }}
    </style>
</head>
<body>
    <div class="card">
        <div class="icon">&#9888;</div>
        <h1>Something went wrong</h1>
        <p>{escaped_message}</p>
    </div>
</body>
</html>"#
    )
}

/// Query params for the MCP browser-OAuth callback (`/oauth/mcp/callback`).
#[derive(Debug, serde::Deserialize)]
struct OAuthMcpCallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// Loopback redirect target for MCP browser OAuth (RFC 8252). The authorization
/// server redirects the browser here with `?code=…&state=…`; we hand it to
/// `mcp_registry::oauth::complete`, which exchanges the code for a token, stores
/// it as the server's `Authorization` header, and reconnects.
async fn oauth_mcp_callback_handler(
    Query(query): Query<OAuthMcpCallbackQuery>,
) -> impl IntoResponse {
    let html = |status: StatusCode, body: String| -> Response {
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response()
    };

    if let Some(err) = query.error.as_deref().filter(|s| !s.is_empty()) {
        let desc = query.error_description.as_deref().unwrap_or("");
        log::warn!("[oauth:mcp] authorization error: {err} {desc}");
        return html(
            StatusCode::BAD_REQUEST,
            error_html(&format!("Authorization was denied or failed: {err} {desc}")),
        );
    }

    let (code, state) = match (
        query
            .code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        query
            .state
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
    ) {
        (Some(c), Some(s)) => (c.to_string(), s.to_string()),
        _ => {
            return html(
                StatusCode::BAD_REQUEST,
                error_html("Missing authorization code or state in the callback."),
            )
        }
    };

    log::info!("[oauth:mcp] callback received (state present); completing exchange");

    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            log::error!("[oauth:mcp] config load failed: {e}");
            return html(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Internal error loading config. Please try again."),
            );
        }
    };

    match crate::openhuman::mcp_registry::oauth::complete(&config, &state, &code).await {
        Ok(server_id) => {
            log::info!("[oauth:mcp] completed sign-in for server_id={server_id}");
            html(
                StatusCode::OK,
                success_html("Signed in. The MCP server is now connected — you can close this tab and return to OpenHuman."),
            )
        }
        Err(e) => {
            log::error!("[oauth:mcp] complete failed: {e}");
            html(
                StatusCode::BAD_GATEWAY,
                error_html(&format!("Sign-in could not be completed: {e}")),
            )
        }
    }
}

/// Require desktop `/auth` callbacks to be top-level document navigations when
/// browser fetch-metadata headers are present.
///
/// The preferred Tauri loopback listener has a per-login state nonce. This
/// legacy core fallback cannot rely on that state, so it must reject embedded
/// resource loads (`<img>`, iframe, fetch, script) before token exchange.
fn desktop_callback_navigation_ok(headers: &axum::http::HeaderMap) -> Result<(), &'static str> {
    let get_str = |name: &str| -> Option<&str> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    };

    if let Some(mode) = get_str("sec-fetch-mode") {
        if mode != "navigate" {
            return Err("Sec-Fetch-Mode must be 'navigate'");
        }
    }

    if let Some(dest) = get_str("sec-fetch-dest") {
        if dest != "document" {
            return Err("Sec-Fetch-Dest must be 'document'");
        }
    }

    Ok(())
}

/// Inspect the browser fetch-metadata + Referer/Origin headers and decide
/// whether the inbound `/auth/telegram` request looks like a legitimate
/// top-level redirect from Telegram, or a cross-site CSRF attempt.
///
/// The endpoint cannot require a bearer token (the redirect happens in a
/// fresh browser tab; `EventSource`-style header injection is not an
/// option), and there is no in-process state issued by an authenticated
/// FE flow today (`/start register` is initiated in Telegram, not in the
/// local app). So this fetch-metadata gate is the layer that distinguishes
/// "user clicked the link the bot sent them" from "malicious page
/// navigates the user's loopback core via `window.location`/`<img>`".
///
/// Accepted shapes:
/// - All `Sec-Fetch-*` headers absent (older browsers, CLI clients).
/// - `Sec-Fetch-Mode: navigate` AND `Sec-Fetch-Dest: document`.
/// - `Sec-Fetch-Site` is `same-origin` / `none`, OR `cross-site` with a
///   `Referer` that starts with `https://t.me/` (the legit bot redirect).
///
/// Rejected shapes:
/// - `Sec-Fetch-Mode` is `no-cors` / `cors` / `same-origin` (only
///   `navigate` makes sense for a top-level page load).
/// - `Sec-Fetch-Dest` is anything other than `document` (image/script/
///   iframe embeds from malicious pages).
/// - `Sec-Fetch-Site: cross-site` with a `Referer`/`Origin` that is not
///   `https://t.me/...` (CSRF redirect from a third-party site).
fn telegram_callback_origin_ok(headers: &axum::http::HeaderMap) -> Result<(), &'static str> {
    let get_str = |name: &str| -> Option<&str> {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    };

    let mode = get_str("sec-fetch-mode");
    let dest = get_str("sec-fetch-dest");
    let site = get_str("sec-fetch-site");
    let referer = get_str("referer");
    let origin = get_str("origin");

    if let Some(mode) = mode {
        if mode != "navigate" {
            return Err("Sec-Fetch-Mode must be 'navigate'");
        }
    }
    if let Some(dest) = dest {
        if dest != "document" {
            return Err("Sec-Fetch-Dest must be 'document'");
        }
    }

    let referer_is_telegram = referer
        .map(|r| r.starts_with("https://t.me/") || r.starts_with("https://web.telegram.org/"))
        .unwrap_or(false);
    let origin_is_telegram = origin
        .map(|o| o == "https://t.me" || o == "https://web.telegram.org")
        .unwrap_or(false);

    if let Some(site) = site {
        if site == "cross-site" && !(referer_is_telegram || origin_is_telegram) {
            return Err("cross-site redirect must originate from telegram");
        }
    } else if let Some(referer) = referer {
        // No Sec-Fetch-Site: fall back to Referer host check. Accept
        // loopback referer (direct nav inside the local app) — parsed
        // exactly so `http://localhost.attacker.example/...` does not
        // satisfy the gate — and accept telegram referer (legit bot
        // redirect); reject everything else.
        let local = url::Url::parse(referer)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .map(|h| matches!(h.as_str(), "localhost" | "127.0.0.1" | "::1"))
            .unwrap_or(false);
        if !(local || referer_is_telegram) {
            return Err("Referer must be telegram or local");
        }
    }

    Ok(())
}

/// Handles the Telegram authentication callback.
///
/// It consumes a one-time token, exchanges it for a JWT from the backend,
/// and stores the session locally.
async fn telegram_auth_handler(
    headers: axum::http::HeaderMap,
    Query(query): Query<TelegramAuthQuery>,
) -> impl IntoResponse {
    let html_response = |status: StatusCode, body: String| -> Response {
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response()
    };

    if let Err(reason) = telegram_callback_origin_ok(&headers) {
        log::warn!("[auth:telegram] rejecting callback: {reason}");
        return html_response(
            StatusCode::FORBIDDEN,
            error_html(
                "This login callback did not come from the Telegram bot. \
                 Open the link the bot sent you directly, do not let \
                 another page redirect you here.",
            ),
        );
    }

    let token = match query
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(t) => t.to_string(),
        None => {
            return html_response(
                StatusCode::BAD_REQUEST,
                error_html("Missing token parameter. Send /start register to the bot again."),
            )
        }
    };

    log::info!("[auth:telegram] Received registration callback with token");

    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            log::error!("[auth:telegram] Failed to load config: {e}");
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Internal error. Please try again."),
            );
        }
    };

    let api_url = crate::api::config::effective_backend_api_url(&config.api_url);

    let client = match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => c,
        Err(e) => {
            log::error!("[auth:telegram] Failed to create API client: {e}");
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Internal error. Please try again."),
            );
        }
    };

    // Exchange the login token for a session JWT.
    let jwt_token = match client.consume_login_token(&token).await {
        Ok(jwt) => jwt,
        Err(e) => {
            let error_str = e.to_string();
            // Check if this is a client-side error (token validation) or server-side error
            let is_client_error = error_str.contains("expired")
                || error_str.contains("invalid")
                || error_str.contains("not found")
                || error_str.contains("already used")
                || error_str.contains("401")
                || error_str.contains("400")
                || error_str.contains("404");

            if is_client_error {
                log::warn!("[auth:telegram] Token consumption failed (client error): {e}");
                return html_response(
                    StatusCode::BAD_REQUEST,
                    error_html(
                        "This link has expired or was already used. Send /start register to the bot again.",
                    ),
                );
            } else {
                log::error!("[auth:telegram] Token consumption failed (server error): {e}");
                return html_response(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    error_html("Internal server error, please try again later."),
                );
            }
        }
    };

    // Store the resulting session token in the local configuration.
    match crate::openhuman::credentials::ops::store_session_with_deferred_validation(
        &config, &jwt_token, None, None,
    )
    .await
    {
        Ok(outcome) => {
            for msg in &outcome.logs {
                log::info!("[auth:telegram] {msg}");
            }
            log::info!("[auth:telegram] Session stored successfully");
        }
        Err(e) => {
            log::error!("[auth:telegram] Failed to store session: {e}");
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Connected to Telegram but failed to save session. Please try again."),
            );
        }
    }

    html_response(
        StatusCode::OK,
        success_html(
            "Your Telegram account has been connected to OpenHuman. You can close this tab.",
        ),
    )
}

/// Handles the generic desktop login callback fallback.
///
/// The preferred path is the `openhuman://auth?...` deep link handled in the
/// renderer. On hosts where URL-scheme registration is broken, some login
/// flows can fall back to the local core callback (`/auth`). This route is
/// public because the callback carries its own one-time login token; raw
/// session JWT callbacks are intentionally rejected on this public surface.
async fn desktop_auth_handler(
    headers: axum::http::HeaderMap,
    Query(query): Query<DesktopAuthQuery>,
) -> impl IntoResponse {
    let html_response = |status: StatusCode, body: String| -> Response {
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response()
    };

    if let Err(reason) = desktop_callback_navigation_ok(&headers) {
        log::warn!("[auth:desktop] Rejected non-navigation callback: {reason}");
        return html_response(
            StatusCode::BAD_REQUEST,
            error_html("Sign-in callback must be opened as a browser page. Please try again."),
        );
    }

    let token = match query
        .token
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(t) => t.to_string(),
        None => {
            return html_response(
                StatusCode::BAD_REQUEST,
                error_html("Sign-in callback was missing a token. Please try again."),
            )
        }
    };

    if query
        .key
        .as_deref()
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .is_some()
    {
        log::warn!("[auth:desktop] Rejected deprecated direct session token callback");
        return html_response(
            StatusCode::BAD_REQUEST,
            error_html("This sign-in callback is no longer supported. Please start sign-in again."),
        );
    }

    log::info!("[auth:desktop] Received desktop auth callback");

    let config = match crate::openhuman::config::Config::load_or_init().await {
        Ok(c) => c,
        Err(e) => {
            log::error!("[auth:desktop] Failed to load config: {e}");
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Internal error. Please try again."),
            );
        }
    };

    let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
    let client = match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => c,
        Err(e) => {
            log::error!("[auth:desktop] Failed to create API client: {e}");
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html("Internal error. Please try again."),
            );
        }
    };

    let jwt_token = match client.consume_login_token(&token).await {
        Ok(jwt) => jwt,
        Err(e) => {
            log::warn!("[auth:desktop] Login token consumption failed: {e}");
            return html_response(
                StatusCode::BAD_REQUEST,
                error_html("This sign-in link has expired or was already used. Please try again."),
            );
        }
    };

    match crate::openhuman::credentials::ops::store_session_with_deferred_validation(
        &config, &jwt_token, None, None,
    )
    .await
    {
        Ok(outcome) => {
            for msg in &outcome.logs {
                log::info!("[auth:desktop] {msg}");
            }
            log::info!("[auth:desktop] Session stored successfully");
        }
        Err(e) => {
            log::error!("[auth:desktop] Failed to store session: {e}");
            return html_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                error_html(
                    "Sign-in succeeded but OpenHuman could not save the session. Please try again.",
                ),
            );
        }
    }

    html_response(
        StatusCode::OK,
        success_html("Sign-in completed. You can close this tab and return to OpenHuman."),
    )
}

/// Query parameters for the dictation WebSocket endpoint.
///
/// Browser `WebSocket` cannot attach an `Authorization` header on upgrade, so
/// the FE forwards the per-process core bearer as a `?token=…` query param —
/// validated against the same in-process RPC token via [`verify_bearer_token`]
/// (single source of truth, no separate credential).
#[derive(Debug, serde::Deserialize)]
struct DictationQuery {
    #[serde(default)]
    token: Option<String>,
}

/// WebSocket upgrade handler for streaming voice dictation.
///
/// Authenticated before upgrade (C4 / issue #1924): the request must carry the
/// per-process core bearer either as `Authorization: Bearer <token>` (CLI /
/// native callers) or as `?token=<token>` (browser `WebSocket`, which cannot
/// set headers), and — when an `Origin` header is present — that origin must be
/// on the local-app allowlist, mirroring the Socket.IO handshake check. Missing
/// or wrong credentials are rejected with 401 and the socket is never upgraded.
async fn dictation_ws_handler(
    headers: axum::http::HeaderMap,
    Query(query): Query<DictationQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    log::info!("[ws] dictation WebSocket upgrade requested");

    // Origin check (same allowlist Socket.IO enforces): native clients send no
    // Origin and are accepted; cross-origin browser pages are rejected even if
    // they somehow hold the bearer.
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::trim);
    if !crate::core::socketio::origin_is_allowed(origin) {
        log::warn!("[ws] dictation upgrade rejected: disallowed origin {origin:?}");
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "ok": false,
                "error": "forbidden",
                "message": "Origin not allowed for the dictation WebSocket."
            })),
        )
            .into_response();
    }

    // Bearer check: header first, then `?token=` for browser WebSocket clients.
    let header_token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let bearer_ok = header_token
        .map(crate::core::auth::verify_bearer_token)
        .unwrap_or(false);
    let bearer_ok = bearer_ok
        || query
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(crate::core::auth::verify_bearer_token)
            .unwrap_or(false);
    if !bearer_ok {
        log::warn!("[ws] dictation upgrade rejected: missing or invalid bearer token");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": "unauthorized",
                "message": "Missing or invalid token. Supply 'Authorization: Bearer <core>' or ?token=<core>."
            })),
        )
            .into_response();
    }

    ws.on_upgrade(|socket| async move {
        let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                log::error!("[ws] failed to load config for dictation: {e}");
                return;
            }
        };
        crate::openhuman::voice::streaming::handle_dictation_ws(socket, config).await;
    })
}

/// Maximum accepted request-body size for the core HTTP server (64 MiB).
///
/// Sized to comfortably hold a `channel_web_chat` turn carrying the composer's
/// maximum image payload — 4 × 8 MiB raw ≈ 43 MiB once base64-encoded into
/// `[IMAGE:data:…]` markers — plus message text and JSON-RPC envelope overhead.
/// Axum's 2 MiB default would otherwise reject any image attachment (#3205).
const MAX_RPC_BODY_BYTES: usize = 64 * 1024 * 1024;

/// Builds the main Axum router for the core HTTP server.
///
/// Includes routes for health, schema, SSE events, JSON-RPC, and Telegram auth.
/// Conditionally attaches Socket.IO if enabled.
///
/// Middleware order (outermost → innermost):
/// 1. `cors_middleware`       — handles `OPTIONS` preflight and adds CORS headers
/// 2. `rpc_auth_middleware`   — validates `Authorization: Bearer <token>` on protected paths
/// 3. `http_request_log_middleware` — logs non-RPC HTTP requests with timing
pub fn build_core_http_router(socketio_enabled: bool) -> Router {
    let mut router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/schema", get(schema_handler))
        .route("/events", get(events_handler))
        .route("/events/webhooks", get(webhook_events_handler))
        .route("/events/domain", get(domain_events_handler))
        // Raise the request-body cap above Axum's 2 MiB default — scoped to
        // `/rpc` only so other routes keep the default. Chat image attachments
        // are inlined into the `channel_web_chat` JSON-RPC body as base64
        // `data:` URIs, and the composer permits up to ATTACHMENT_MAX_IMAGES (4)
        // × ATTACHMENT_MAX_SIZE_BYTES (8 MiB) of raw image ≈ 43 MiB once
        // base64-encoded. Without this the whole turn was rejected at the local
        // RPC boundary with "failed to buffer the request body: length limit
        // exceeded" before anything reached the provider (issue #3205). The
        // server binds to 127.0.0.1 behind a per-launch bearer, so a generous
        // localhost cap is safe.
        .route(
            "/rpc",
            post(rpc_handler).route_layer(DefaultBodyLimit::max(MAX_RPC_BODY_BYTES)),
        )
        .route("/ws/dictation", get(dictation_ws_handler))
        .route("/auth", get(desktop_auth_handler))
        .route("/auth/telegram", get(telegram_auth_handler))
        .route("/oauth/mcp/callback", get(oauth_mcp_callback_handler))
        // OpenAI-compatible inference endpoint (/v1/chat/completions, /v1/models)
        .nest("/v1", crate::openhuman::inference::http::router())
        // Apply `AppState` here (before any state-less sub-routers such as
        // AgentBox are merged below) so the outer router becomes
        // `Router<()>` and matches them.
        .with_state(AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        });

    // Mount AgentBox marketplace routes when explicitly enabled.
    //
    // Gate is strict literal "1" — "true"/"yes"/etc. do NOT enable it. Auth
    // bypass for `/run` and `/jobs/{id}` is unconditional in
    // [`crate::core::auth`]; the router-side gate is what actually exposes
    // the handlers. The spawned sweep loop lives until process exit.
    if crate::openhuman::agentbox::agentbox_mode_enabled() {
        let store = crate::openhuman::agentbox::JobStore::new(std::time::Duration::from_secs(3600));
        let invoker: std::sync::Arc<dyn crate::openhuman::agentbox::invoker::AgentInvoker> =
            std::sync::Arc::new(crate::openhuman::agentbox::invoker::CoreAgentInvoker);
        let job_timeout = std::env::var("OPENHUMAN_AGENTBOX_JOB_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(std::time::Duration::from_secs)
            .unwrap_or_else(|| std::time::Duration::from_secs(600));

        // Spawn sweep loop — bounds memory under sustained traffic.
        let sweep_store = store.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                tick.tick().await;
                let evicted = sweep_store.sweep_now();
                if evicted > 0 {
                    log::info!("[agentbox] sweep evicted {} terminal jobs", evicted);
                }
            }
        });

        log::info!("[agentbox] enabled; public routes: POST /run, GET /jobs/{{id}}, GET /health");
        router = router.merge(crate::openhuman::agentbox::agentbox_router(
            store,
            invoker,
            job_timeout,
        ));
    }

    let router = router
        .fallback(not_found_handler)
        .layer(middleware::from_fn(http_request_log_middleware))
        .layer(middleware::from_fn(crate::core::auth::rpc_auth_middleware))
        .layer(middleware::from_fn(cors_middleware));

    if socketio_enabled {
        let (socket_layer, io) = crate::core::socketio::attach_socketio();
        crate::core::socketio::spawn_web_channel_bridge(io);
        return router.layer(socket_layer);
    }

    router
}

/// Middleware for logging incoming HTTP requests.
///
/// The `/rpc` path is logged inside [`rpc_handler`] instead (with the
/// JSON-RPC method name), so we skip it here to avoid a redundant line.
async fn http_request_log_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query_len = req.uri().query().map(str::len).unwrap_or(0);
    let started = std::time::Instant::now();

    let response = next.run(req).await;

    if path != "/rpc" {
        let status = response.status().as_u16();
        let ms = started.elapsed().as_millis();
        tracing::info!(
            "[http] {} {}{} -> {} ({}ms)",
            method,
            path,
            if query_len > 0 { "?…" } else { "" },
            status,
            ms
        );
    }

    response
}

/// Environment variable for additional comma-separated origins to allow.
/// Intended for debug harnesses and E2E setups that don't run on loopback —
/// e.g. `OPENHUMAN_CORE_ALLOWED_ORIGINS=https://e2e.internal,http://my-debugger:8080`.
const ALLOWED_ORIGINS_ENV: &str = "OPENHUMAN_CORE_ALLOWED_ORIGINS";

/// Decides whether a browser `Origin` header value is allowed to make
/// authenticated cross-origin requests against the local RPC server.
///
/// The RPC server only ever serves three legitimate consumers:
///   1. The bundled Tauri v2 webview — `tauri://localhost` on macOS/Linux and
///      `http(s)://tauri.localhost` on Windows.
///   2. The Vite dev server during `pnpm dev` — any port on loopback hosts.
///   3. Operator-controlled debug harnesses opted in via
///      `OPENHUMAN_CORE_ALLOWED_ORIGINS`.
///
/// Anything else (a random web page that has somehow obtained the bearer
/// token via leaked logs / screenshots / a compromised third-party origin
/// loaded in a CEF child webview) must be refused — the bearer token alone
/// is not enough authorization without an origin binding.
pub(super) fn is_origin_allowed(origin: &str) -> bool {
    let extra_origins = std::env::var(ALLOWED_ORIGINS_ENV).ok();
    is_origin_allowed_with_extra(origin, extra_origins.as_deref())
}

pub(super) fn is_origin_allowed_with_extra(origin: &str, extra_origins: Option<&str>) -> bool {
    // Tauri v2 webview origins. Windows uses an HTTP(S) custom host; macOS
    // and Linux use the `tauri://` scheme. We accept both for portability.
    if matches!(
        origin,
        "tauri://localhost" | "http://tauri.localhost" | "https://tauri.localhost"
    ) {
        return true;
    }

    // Loopback origins on any port (Vite dev server, E2E driver, CLI tools).
    if let Some(rest) = origin.strip_prefix("http://") {
        let authority = rest.split('/').next().unwrap_or("");
        let host = if let Some(stripped) = authority.strip_prefix('[') {
            // IPv6 literal: `[::1]:1420` → `::1`
            stripped.split(']').next().unwrap_or("")
        } else {
            authority.split(':').next().unwrap_or("")
        };
        if matches!(host, "127.0.0.1" | "localhost" | "::1") {
            return true;
        }
    }

    // Env override: comma-separated exact matches.
    if let Some(extra) = extra_origins {
        for candidate in extra.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            if candidate == origin {
                return true;
            }
        }
    }

    false
}

/// Middleware for handling Cross-Origin Resource Sharing (CORS).
///
/// Reads the request's `Origin` header before invoking the inner handler so
/// the same value can be echoed back (when allowed) on the response.
async fn cors_middleware(req: Request, next: Next) -> Response {
    let origin = req
        .headers()
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    if req.method() == Method::OPTIONS {
        return with_cors_headers(StatusCode::NO_CONTENT.into_response(), origin.as_deref());
    }

    let response = next.run(req).await;
    with_cors_headers(response, origin.as_deref())
}

/// Injects CORS headers into a response.
///
/// If the request carried an `Origin` header and that origin is on the
/// allowlist, the value is echoed back in `Access-Control-Allow-Origin` and
/// `Vary: Origin` is set so intermediate caches keep per-origin responses
/// distinct. Disallowed origins receive no `Access-Control-Allow-Origin`
/// header at all — the browser will then refuse to surface the response to
/// the calling JS. Non-browser callers (no `Origin` header) are unaffected.
///
/// For Docker / cloud deployments where the server binds to `0.0.0.0`,
/// extend the allowlist via the `OPENHUMAN_CORE_ALLOWED_ORIGINS` env var
/// (comma-separated) rather than wildcarding `Access-Control-Allow-Origin`.
pub(super) fn with_cors_headers(mut response: Response, origin: Option<&str>) -> Response {
    let headers = response.headers_mut();
    headers.append(header::VARY, HeaderValue::from_static("Origin"));

    if let Some(o) = origin {
        if is_origin_allowed(o) {
            if let Ok(val) = HeaderValue::from_str(o) {
                headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, val);
            }
        } else {
            tracing::warn!("[cors] rejected disallowed origin: {}", o);
        }
    }

    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Authorization"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    response
}

/// Handler for the health check endpoint.
///
/// Liveness is granular (#3312): a single degraded *background* component
/// (scheduler, channels, update_checker, …) no longer 503s the whole container.
/// `/health` returns 503 only when a *critical* component is unhealthy (see
/// `health::CRITICAL_COMPONENTS`); otherwise it returns 200 — with a `degraded`
/// flag and per-component buckets in the body so readiness probes and operators
/// can still see partial failures.
async fn health_handler() -> impl IntoResponse {
    let snapshot = crate::openhuman::health::snapshot();
    let verdict = crate::openhuman::health::verdict(&snapshot);

    let status = if verdict.healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    // Augment the snapshot body with the verdict so the components map stays
    // backward-compatible while exposing overall liveness/readiness.
    let mut body = serde_json::to_value(&snapshot).unwrap_or_else(|_| serde_json::json!({}));
    if let Some(obj) = body.as_object_mut() {
        obj.insert("healthy".to_string(), serde_json::json!(verdict.healthy));
        obj.insert("degraded".to_string(), serde_json::json!(verdict.degraded));
        obj.insert(
            "critical_unhealthy".to_string(),
            serde_json::json!(verdict.critical_unhealthy),
        );
        obj.insert(
            "degraded_components".to_string(),
            serde_json::json!(verdict.degraded_components),
        );
    }

    tracing::debug!(
        "[health] status={} components={} healthy={} degraded={} critical_unhealthy={:?} degraded_components={:?}",
        status.as_u16(),
        snapshot.components.len(),
        verdict.healthy,
        verdict.degraded,
        verdict.critical_unhealthy,
        verdict.degraded_components,
    );

    (status, Json(body))
}

/// Handler for the schema discovery endpoint.
async fn schema_handler(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(build_http_schema_dump())).into_response()
}

/// Query parameters for the events SSE endpoint.
///
/// `client_id` selects which broadcast events to forward; `token` is the
/// single-shot bind token minted by the `core.events_subscribe_token` RPC.
/// Both are required — browser `EventSource` cannot attach an
/// `Authorization` header, so the bind token is the only credential the
/// endpoint accepts.
#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    client_id: String,
    #[serde(default)]
    token: Option<String>,
}

/// Handler for the main events SSE endpoint.
///
/// Accepts either of two credentials:
/// 1. `Authorization: Bearer <core token>` — used by CLI tooling, the
///    Tauri shell via `core_rpc_relay`, and the in-tree e2e suite that
///    can set HTTP headers directly. Validated against the same
///    per-process bearer the rest of `/rpc` uses.
/// 2. `?token=<bind>` minted via the `core.events_subscribe_token` RPC
///    — used by browser `EventSource`, which cannot attach custom
///    headers. The token is bound to a specific `client_id` and is
///    consumed on validation so a leaked URL cannot be replayed.
///
/// Both paths converge on the same broadcast stream filtered by
/// `client_id`.
async fn events_handler(
    headers: axum::http::HeaderMap,
    Query(query): Query<EventsQuery>,
) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let bearer_ok = bearer
        .map(crate::core::auth::verify_bearer_token)
        .unwrap_or(false);

    if !bearer_ok {
        let supplied_token = query
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(supplied_token) = supplied_token else {
            log::warn!(
                "[events] reject subscribe: missing bind token + missing bearer (client_id_len={})",
                query.client_id.len()
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "ok": false,
                    "error": "unauthorized",
                    "message": "Missing credentials. Supply 'Authorization: Bearer <core>' or mint a bind token with the `core.events_subscribe_token` RPC and pass it as ?token="
                })),
            )
                .into_response();
        };
        if !crate::core::event_bind_tokens::consume(&query.client_id, supplied_token) {
            log::warn!(
                "[events] reject subscribe: bind token invalid or expired (client_id_len={})",
                query.client_id.len()
            );
            return (
                StatusCode::UNAUTHORIZED,
                Json(json!({
                    "ok": false,
                    "error": "unauthorized",
                    "message": "Bind token is unknown, expired, or bound to a different client_id."
                })),
            )
                .into_response();
        }
    }

    let client_id = query.client_id;
    let rx = crate::openhuman::channels::providers::web::subscribe_web_channel_events();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        move |item| -> Option<Result<Event, std::convert::Infallible>> {
            let event = match item {
                Ok(ev) => ev,
                Err(_) => return None,
            };
            if event.client_id != client_id {
                return None;
            }
            let data = match serde_json::to_string(&event) {
                Ok(data) => data,
                Err(_) => return None,
            };
            Some(Ok(Event::default().event(event.event).data(data)))
        },
    );

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
        .into_response()
}

/// Handler for the webhook debug events SSE endpoint.
async fn webhook_events_handler() -> Response {
    let stream = tokio_stream::once(Ok::<Event, std::convert::Infallible>(
        Event::default()
            .event("webhooks_debug")
            .data("{\"event_type\":\"runtime_removed\"}"),
    ));
    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
        .into_response()
}

/// SSE endpoint streaming DomainEvent bus events for the live event log panel.
///
/// Requires bearer auth. Streams all domain events as JSON with event type
/// set to the domain name (agent, tool, memory, etc.).
async fn domain_events_handler(headers: axum::http::HeaderMap) -> Response {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let bearer_ok = bearer
        .map(crate::core::auth::verify_bearer_token)
        .unwrap_or(false);

    if !bearer_ok {
        log::warn!("[events/domain] reject subscribe: missing or invalid bearer token");
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({
                "ok": false,
                "error": "unauthorized",
                "message": "Bearer token required for domain event stream"
            })),
        )
            .into_response();
    }

    // Read dashboard config for event stream settings.
    let es_cfg = crate::openhuman::config::rpc::load_config_with_timeout()
        .await
        .map(|c| c.dashboard.event_stream)
        .unwrap_or_default();

    if !es_cfg.enabled {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": "event stream disabled by config" })),
        )
            .into_response();
    }

    let bus = match crate::core::event_bus::global() {
        Some(bus) => bus,
        None => {
            log::warn!("[events/domain] event bus not initialized");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(json!({ "ok": false, "error": "event bus not initialized" })),
            )
                .into_response();
        }
    };

    log::debug!("[events/domain] client connected, streaming domain events");

    // Send config as first SSE event so frontend can apply settings.
    let config_event = Event::default().event("config").data(
        serde_json::to_string(&json!({
            "max_entries": es_cfg.max_entries,
            "new_entries": es_cfg.new_entries,
        }))
        .unwrap_or_default(),
    );

    let rx = bus.raw_receiver();
    let event_stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        |item| -> Option<Result<Event, std::convert::Infallible>> {
            let event = match item {
                Ok(ev) => ev,
                Err(_) => return None,
            };
            let domain = event.domain().to_string();
            let event_name = event.variant_name();
            let agent = event.agent_hint().unwrap_or("").to_string();
            let data = json!({
                "domain": domain,
                "event": event_name,
                "agent": agent,
                "timestamp": chrono::Utc::now().format("%H:%M:%S").to_string(),
            });
            let data_str = serde_json::to_string(&data).ok()?;
            Some(Ok(Event::default().event(domain).data(data_str)))
        },
    );

    let config_stream =
        futures::stream::once(async move { Ok::<_, std::convert::Infallible>(config_event) });
    let stream = config_stream.chain(event_stream);

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(5)))
        .into_response()
}

/// Handler for the root endpoint, returning server information and available endpoints.
async fn root_handler() -> impl IntoResponse {
    let api_server = match crate::openhuman::config::Config::load_or_init().await {
        Ok(cfg) => crate::api::config::effective_backend_api_url(&cfg.api_url),
        Err(_) => crate::api::config::effective_backend_api_url(&None),
    };

    (
        StatusCode::OK,
        Json(json!({
            "name": "openhuman",
            "ok": true,
            "api_server": api_server,
            "endpoints": {
                "health": "/health",
                "schema": "/schema",
                "events": "/events?client_id=<id>&token=<core.events_subscribe_token>",
                "rpc": "/rpc"
            },
            "usage": {
                "jsonrpc": {
                    "version": "2.0",
                    "method": "core.ping",
                    "params": {}
                }
            }
        })),
    )
}

/// Fallback handler for unknown routes.
async fn not_found_handler() -> impl IntoResponse {
    (
        StatusCode::NOT_FOUND,
        Json(json!({
            "ok": false,
            "error": "not_found",
            "message": "Route not found. Try /, /health, /schema, or /rpc."
        })),
    )
}

/// Resolves the port for the core server from environment variables or defaults.
pub(crate) fn core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

/// Resolves the bind address host for the core server from environment variables or defaults.
pub(crate) fn core_host() -> String {
    std::env::var("OPENHUMAN_CORE_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

/// Metadata sent back to the Tauri host once the embedded core has selected
/// and bound its listen port.
#[derive(Debug, Clone)]
pub struct EmbeddedReadySignal {
    pub port: u16,
    pub fallback_from: Option<u16>,
}

/// Runs the HTTP/JSON-RPC server.
///
/// This function binds to the specified host and port, initializes the router,
/// bootstraps long-lived runtime infrastructure, and starts serving requests.
pub async fn run_server(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
) -> anyhow::Result<()> {
    run_server_inner(host, port, socketio_enabled, false, None, None, None).await
}

/// Runs the request/response-only HTTP API without detached background jobs.
pub async fn run_server_headless(host: Option<&str>, port: Option<u16>) -> anyhow::Result<()> {
    let services = crate::core::runtime::ServiceSet::headless_api();
    run_server_with_services(host, port, services, false, None, None, None).await
}

/// Like [`run_server`] but marks the instance as embedded.
pub async fn run_server_embedded(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
    shutdown_token: CancellationToken,
) -> anyhow::Result<()> {
    run_server_inner(
        host,
        port,
        socketio_enabled,
        true,
        Some(shutdown_token),
        None,
        None,
    )
    .await
}

/// Embedded entrypoint with an explicit readiness callback.
///
/// When the caller already holds the per-launch RPC bearer in memory (the
/// Tauri shell now that the core runs in-process — PR #1061), it should
/// pass `Some(token)` so the embedded server can seed its auth subsystem
/// via [`crate::core::auth::init_rpc_token_with_value`] without ever
/// reading `OPENHUMAN_CORE_TOKEN` from the process environment.  Passing
/// `None` preserves the env-as-config fallback (CLI / docker / cloud).
pub async fn run_server_embedded_with_ready(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
    shutdown_token: CancellationToken,
    ready_tx: tokio::sync::oneshot::Sender<EmbeddedReadySignal>,
    rpc_token: Option<std::sync::Arc<String>>,
) -> anyhow::Result<()> {
    run_server_inner(
        host,
        port,
        socketio_enabled,
        true,
        Some(shutdown_token),
        Some(ready_tx),
        rpc_token,
    )
    .await
}

/// Internal server entrypoint.
async fn run_server_inner(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
    embedded_core: bool,
    shutdown_token: Option<CancellationToken>,
    ready_tx: Option<tokio::sync::oneshot::Sender<EmbeddedReadySignal>>,
    rpc_token: Option<std::sync::Arc<String>>,
) -> anyhow::Result<()> {
    let mut services = crate::core::runtime::ServiceSet::desktop();
    services.socketio = socketio_enabled;
    run_server_with_services(
        host,
        port,
        services,
        embedded_core,
        shutdown_token,
        ready_tx,
        rpc_token,
    )
    .await
}

async fn run_server_with_services(
    host: Option<&str>,
    port: Option<u16>,
    services: crate::core::runtime::ServiceSet,
    embedded_core: bool,
    shutdown_token: Option<CancellationToken>,
    ready_tx: Option<tokio::sync::oneshot::Sender<EmbeddedReadySignal>>,
    rpc_token: Option<std::sync::Arc<String>>,
) -> anyhow::Result<()> {
    // `run_server_inner` is now a thin shim over the CoreBuilder/CoreRuntime
    // composition (Phase 1). It reproduces the legacy behavior exactly: all
    // background services on (`ServiceSet::desktop`), Socket.IO per the caller
    // flag, and the legacy `embedded_core` → `HostKind` mapping (embedded ==
    // Tauri shell; standalone splits CLI / Docker via `detect_standalone`).
    // See `docs/plans/pluggable-core/phase-1-corebuilder.md`.
    let host_kind = if embedded_core {
        crate::core::types::HostKind::TauriShell
    } else {
        crate::core::types::HostKind::detect_standalone()
    };
    let token = match rpc_token {
        Some(token) => crate::core::runtime::TokenSource::Fixed(token),
        None => crate::core::runtime::TokenSource::EnvOrFile,
    };
    let mut builder = crate::core::runtime::CoreBuilder::new(host_kind)
        .token(token)
        .services(services);
    if let Some(host) = host {
        builder = builder.host(host);
    }
    if let Some(port) = port {
        builder = builder.port(port);
    }

    let runtime = builder.build().await?;
    runtime.serve(ready_tx, shutdown_token).await
}

/// Per-`DomainGroup` gating decision for each event-bus subscriber that
/// [`register_domain_subscribers`] conditionally registers. Extracted as a
/// pure value so the subscriber→group mapping has a single source of truth
/// that the registrar consumes and tests assert directly — without registering
/// real subscribers or touching the process-global event bus (#4796 DoD item 3).
///
/// Unlisted subscribers (health, scheduler-gate, TokenJuice content-router,
/// session-token seeding, `SessionExpired`, service restart/shutdown) are
/// always registered as core/platform infra and intentionally absent here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DomainSubscriberPlan {
    /// webhook + notification-bridge + composio trigger + task-sources + device-tunnel.
    pub platform: bool,
    /// channel-inbound + web-only proactive.
    pub channels: bool,
    /// flows trigger dispatch.
    pub flows: bool,
    /// memory conversation-persistence + sync-stage bridge.
    pub memory: bool,
    /// agent_meetings calendar + meeting-event subscribers.
    pub meet: bool,
    /// agent handlers + background delivery + run-ledger finalizer + orchestration ingest.
    pub agent: bool,
    /// mcp_registry lifecycle bus init.
    pub mcp: bool,
}

impl DomainSubscriberPlan {
    /// The subscriber-registration plan for `domains`. Pure: no side effects.
    pub fn for_domains(domains: crate::core::runtime::DomainSet) -> Self {
        use crate::core::all::DomainGroup;
        Self {
            platform: domains.allows(DomainGroup::Platform),
            channels: domains.allows(DomainGroup::Channels),
            flows: domains.allows(DomainGroup::Flows),
            memory: domains.allows(DomainGroup::Memory),
            meet: domains.allows(DomainGroup::Meet),
            agent: domains.allows(DomainGroup::Agent),
            mcp: domains.allows(DomainGroup::Mcp),
        }
    }
}

/// Registers all long-lived domain event-bus subscribers, each group at most
/// once per process.
///
/// Ungated core/platform infra runs exactly once behind `INFRA: Once`; each
/// gated [`DomainGroup`](crate::core::all::DomainGroup) installs the first time
/// it is enabled (tracked by `group_first_time`), so widening the ambient
/// `DomainSet` on a later call (`harness()` → `full()`) still installs the
/// newly-enabled groups without double-subscribing the ones already registered.
fn register_domain_subscribers(
    workspace_dir: std::path::PathBuf,
    config: crate::openhuman::config::Config,
    embedded_core: bool,
    domains: crate::core::runtime::DomainSet,
) {
    use crate::core::all::DomainGroup;
    use std::collections::HashSet;
    use std::sync::{Arc, Mutex, Once, OnceLock};

    let plan = DomainSubscriberPlan::for_domains(domains);
    log::debug!("[event_bus] register_domain_subscribers: domains={domains:?} plan={plan:?}");

    // Per-group idempotency (#4808 review): the previous single process-wide
    // `Once` fixed the subscriber set to the FIRST caller's DomainSet — an
    // embedder or test that built `harness()`/`none()` first and later widened
    // to `full()` would never install the subscribers skipped on that first
    // call, even though those domains' controllers are now exposed. Tracking the
    // set of already-registered groups lets a later, wider DomainSet install
    // exactly the newly-enabled groups (and no group twice). `insert` returns
    // `true` only the first time a group is seen.
    fn group_first_time(group: DomainGroup) -> bool {
        static DONE: OnceLock<Mutex<HashSet<DomainGroup>>> = OnceLock::new();
        DONE.get_or_init(|| Mutex::new(HashSet::new()))
            .lock()
            .expect("domain-subscriber registry lock poisoned")
            .insert(group)
    }

    // Ungated core/platform infra — health, scheduler-gate, TokenJuice
    // content-router, session-token seeding, the SessionExpired handler, and
    // service restart/shutdown. These are DomainSet-independent, so they run
    // exactly once on the first call regardless of which composition boots
    // first. Registered BEFORE any gated subscriber so the SessionExpired
    // handler is live before a gated subscriber could publish a 401-derived
    // event. Leaked `SubscriptionHandle`s live for the whole process
    // (`SubscriptionHandle::drop` aborts the task).
    static INFRA: Once = Once::new();
    INFRA.call_once(|| {
        crate::openhuman::health::bus::register_health_subscriber();

        // Initialise the scheduler gate before any background AI workers start
        // so they observe a real policy on their first iteration (otherwise they
        // fall back to `Policy::Normal` and miss the initial throttle decision on
        // battery-powered hosts).
        crate::openhuman::scheduler_gate::init_global(&config);

        // Install the TokenJuice content-router runtime config (compressor
        // toggles + CCR cache limits + optional on-disk tier). Compaction runs on
        // every agent's tool output, so this must be set before any agent loop
        // executes a tool.
        crate::openhuman::tokenjuice::install_from_config(&config);

        // Seed the scheduler-gate signed-out override from the on-disk session.
        // Without this, a sidecar that boots with no stored JWT would happily
        // spin up cron / channel loops and fire LLM requests that all 401.
        match crate::api::jwt::get_session_token(&config) {
            Ok(Some(_)) => {
                crate::openhuman::scheduler_gate::set_signed_out(false);
            }
            Ok(None) => {
                log::info!(
                    "[auth] no session token at startup — scheduler gate set to signed_out \
                     (config_path={}, keyring_backend={})",
                    config.config_path.display(),
                    crate::openhuman::keyring::backend_name(),
                );
                crate::openhuman::scheduler_gate::set_signed_out(true);
            }
            Err(err) => {
                log::warn!(
                    "[auth] failed to read session token at startup ({err}) — assuming signed_out \
                     (config_path={}, keyring_backend={})",
                    config.config_path.display(),
                    crate::openhuman::keyring::backend_name(),
                );
                crate::openhuman::scheduler_gate::set_signed_out(true);
            }
        }

        // Register the SessionExpired handler before any subscribers that might
        // publish 401-derived events, so the very first 401 is routed through
        // `clear_session` + the scheduler-gate override.
        if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
            crate::openhuman::credentials::bus::SessionExpiredSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!(
                "[event_bus] failed to register SessionExpired subscriber — bus not initialized"
            );
        }

        // Restart requests go through a subscriber so every trigger path shares
        // the same respawn logic.
        crate::openhuman::service::bus::register_restart_subscriber();
        if embedded_core {
            log::info!(
                "[event_bus] embedded core: service shutdown subscriber not registered; Tauri cancellation token owns shutdown"
            );
        } else {
            // Shutdown requests use the same pattern; the standalone CLI
            // subscriber exits the current process after a short grace period.
            crate::openhuman::service::bus::register_shutdown_subscriber();
        }
    });

    // ---- Gated domain subscribers — each group installed at most once, the
    // first time its owning DomainGroup is enabled. -------------------------

    // Platform: webhook + notification bridge + composio trigger + task-sources
    // proactive ingestion + device tunnel.
    if plan.platform {
        if group_first_time(DomainGroup::Platform) {
            if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
                crate::openhuman::webhooks::bus::WebhookRequestSubscriber::new(),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register webhook subscriber — bus not initialized"
                );
            }
            crate::openhuman::notifications::register_notification_bridge_subscriber(
                config.clone(),
            );
            if let Err(error) =
                crate::openhuman::composio::init_composio_trigger_history(workspace_dir.clone())
            {
                log::warn!("[composio][history] failed to initialize trigger archive: {error}");
            }
            crate::openhuman::composio::register_composio_trigger_subscriber();
            crate::openhuman::task_sources::bus::register_task_sources_subscriber();
            // Device tunnel subscriber: handles tunnel:frame handshakes,
            // peer-status events, and register acks. Must be live before any
            // tunnel:frame events can arrive.
            crate::openhuman::devices::bus::register_device_tunnel_subscriber();
        }
    } else {
        log::debug!(
            "[event_bus] Platform subscribers (webhook/notification/composio/task-sources/device-tunnel) SKIPPED — Platform domain disabled"
        );
    }

    // Channels: inbound dispatch + web-only proactive messaging.
    if plan.channels {
        if group_first_time(DomainGroup::Channels) {
            if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
                crate::openhuman::channels::bus::ChannelInboundSubscriber::new(),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register channel subscriber — bus not initialized"
                );
            }
            // Web-only proactive message subscriber (no external channel
            // instances are registered here in the desktop runtime).
            crate::openhuman::channels::proactive::register_web_only_proactive_subscriber();
        }
    } else {
        log::debug!(
            "[event_bus] Channels subscribers (inbound + web-only proactive) SKIPPED — Channels domain disabled"
        );
    }

    // Flows trigger dispatch (issue B2): maps FlowScheduleTick /
    // ComposioTriggerReceived / WebhookIncomingRequest onto enabled flows and
    // runs `flows::ops::flows_run`, so schedule/app-event workflows still
    // dispatch when no realtime channel is configured or
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` short-circuits `start_channels`.
    if plan.flows {
        if group_first_time(DomainGroup::Flows) {
            if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
                crate::openhuman::flows::bus::FlowTriggerSubscriber::new(Arc::new(config.clone())),
            )) {
                std::mem::forget(handle);
            } else {
                log::warn!(
                    "[event_bus] failed to register flows trigger subscriber — bus not initialized"
                );
            }
        }
    } else {
        log::debug!("[event_bus] flows trigger subscriber SKIPPED — Flows domain disabled");
    }

    // Memory: conversation-persistence + sync-stage bridge.
    if plan.memory {
        if group_first_time(DomainGroup::Memory) {
            crate::openhuman::memory_conversations::register_conversation_persistence_subscriber(
                workspace_dir.clone(),
            );
            crate::openhuman::memory::sync::register_sync_stage_bridge(&config);
        }
    } else {
        log::debug!(
            "[event_bus] memory conversation-persistence + sync bridge SKIPPED — Memory domain disabled"
        );
    }

    // Meet: calendar + meeting-event subscribers.
    if plan.meet {
        if group_first_time(DomainGroup::Meet) {
            crate::openhuman::agent_meetings::calendar::register_meet_calendar_subscriber();
            crate::openhuman::agent_meetings::bus::register_meeting_event_subscriber();
        }
    } else {
        log::debug!("[event_bus] agent_meetings subscribers SKIPPED — Meet domain disabled");
    }

    // Agent: orchestration ingest + native agent handlers + background-completion
    // delivery + run-ledger finalizer.
    if plan.agent {
        if group_first_time(DomainGroup::Agent) {
            // Orchestration: ingest tiny.place harness session DMs off the stream bus.
            crate::openhuman::orchestration::register_orchestration_ingest_subscriber();
            // Native request handlers — the agent `agent.run_turn` handler is
            // what channel dispatch calls instead of importing
            // `run_tool_call_loop` directly.
            crate::openhuman::agent::bus::register_agent_handlers();
            // Background-completion delivery: when a detached sub-agent
            // (spawn_async_subagent) finishes, surface its result back into the
            // originating chat as an idle-gated, batched, system-injected turn.
            crate::openhuman::agent_orchestration::background_delivery::register_background_delivery();
            // Run-ledger finalizer: detached `spawn_async_subagent` runs outlive
            // their parent turn, so their terminal `AgentProgress` never reaches
            // the per-turn progress bridge that settles the ledger. This
            // global-bus subscriber settles `agent_runs` from
            // `DomainEvent::Subagent{Completed,Failed}`, preventing rows from
            // leaking as perpetual `running` timeline entries on thread reopen.
            crate::openhuman::agent_orchestration::run_ledger_finalize::register_run_ledger_finalize_subscriber(&config);
        }
    } else {
        log::debug!(
            "[event_bus] agent handlers + background delivery + run-ledger finalizer + orchestration ingest SKIPPED — Agent domain disabled"
        );
    }

    // MCP clients lifecycle subscriber: logs McpServer{Installed,Connected,
    // Disconnected} + McpClientToolExecuted for observability. The boot-time
    // spawn of installed servers (boot::spawn_installed_servers) runs later in
    // bootstrap_core_runtime; this subscriber must be live before then so those
    // connect events are observed (issue #3039 gap A1).
    if plan.mcp {
        if group_first_time(DomainGroup::Mcp) {
            crate::openhuman::mcp_registry::bus::init();
        }
    } else {
        log::debug!("[event_bus] mcp_registry bus init SKIPPED — Mcp domain disabled");
    }

    log::info!("[event_bus] domain subscriber registration complete: plan={plan:?}");
}

/// Initializes long-lived socket/event-bus infrastructure.
///
/// `host_kind` identifies the embedding process (Tauri desktop shell vs
/// standalone CLI / Docker). It drives the approval-gate's host-aware
/// decision tree: under the Tauri shell, the `OPENHUMAN_APPROVAL_GATE=0`
/// env override is ignored and a domain event is published so the UI can
/// surface a banner; under CLI / Docker the override is honored (with a
/// noisy log + a domain event so any connected dashboard can flag it).
pub async fn bootstrap_core_runtime(
    host_kind: crate::core::types::HostKind,
    config: Option<crate::openhuman::config::Config>,
    domains: crate::core::runtime::DomainSet,
) {
    use crate::openhuman::socket::{set_global_socket_manager, SocketManager};
    use std::sync::Arc;
    // `embedded_core` derived from host_kind so the rest of the function (which
    // already keys behavior off the boolean) stays unchanged.
    let embedded_core = host_kind.is_desktop_shell();
    let Some(mut cfg) = config else {
        log::error!(
            "[runtime] Config unavailable for runtime bootstrap; workspace-bound startup skipped"
        );
        return;
    };
    let workspace_dir = cfg.workspace_dir.clone();

    // --- Event bus bootstrap ---
    // Ensure the global event bus is initialized (no-op if already done by start_channels).
    crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
    crate::openhuman::file_state::init_global();
    // Register domain subscribers for cross-module event handling. Ungated infra
    // runs once (INFRA: Once) and each DomainGroup installs at most once via the
    // per-group `group_first_time` set, so repeated calls to
    // bootstrap_core_runtime() cannot double-subscribe (and a later, wider
    // DomainSet still installs its newly-enabled groups).
    register_domain_subscribers(workspace_dir.clone(), cfg.clone(), embedded_core, domains);

    // --- Turn-state recovery -------------------------------------------
    // Any per-thread turn snapshots left on disk from a previous process
    // are stale by definition — there is no live driver to resume them.
    // Stamp them as `Interrupted` so the UI can offer a retry without
    // confusing a stale `Streaming` lifecycle for an in-flight turn.
    {
        let now = chrono::Utc::now().to_rfc3339();
        match crate::openhuman::threads::turn_state::store::mark_all_interrupted(
            workspace_dir.clone(),
            &now,
        ) {
            Ok(0) => {}
            Ok(count) => {
                log::info!("[runtime] marked {count} stale turn snapshot(s) as interrupted")
            }
            Err(err) => {
                log::warn!("[runtime] failed to mark stale turn snapshots interrupted: {err}")
            }
        }
    }

    // --- Run-ledger recovery -------------------------------------------
    // Detached sub-agent runs (`spawn_async_subagent`) from a previous process
    // are gone with that process. Any `agent_runs` row still marked `running`
    // at boot is orphaned — its driver died without firing a terminal event, so
    // the finalizer never settled it. Stamp such rows `interrupted` so they stop
    // rendering as perpetual "running" timeline entries on thread reopen.
    match crate::openhuman::session_db::run_ledger::interrupt_orphaned_agent_runs(&cfg) {
        Ok(0) => {}
        Ok(count) => log::info!("[runtime] settled {count} orphaned agent run(s) on startup"),
        Err(err) => log::warn!("[runtime] failed to settle orphaned agent runs: {err}"),
    }

    // --- Detached sub-agent TaskStore reconciliation -------------------
    // The durable orchestration TaskStore (`<workspace>/.openhuman/
    // orchestration_tasks.jsonl`) can hold non-terminal sub-agent records left
    // by a previous process — their detached executor (abort handle +
    // cooperative CancellationToken) died with that process, so they cannot be
    // re-attached. Reconcile each orphan to a terminal state and emit the typed
    // terminal lifecycle event so the run ledger finalizes. Best-effort and
    // non-fatal (issue #4249 / 07.2 steps 2 & 4).
    {
        let reconciled =
            crate::openhuman::agent_orchestration::running_subagents::reconcile_orphaned_tasks_on_boot(
                &workspace_dir,
            );
        if reconciled > 0 {
            log::info!(
                "[runtime] reconciled {reconciled} orphaned detached sub-agent task(s) on startup"
            );
        }
    }

    // --- Cost dashboard tracker ---
    // Activates the previously-dormant CostTracker so the dashboard RPC
    // surface (`openhuman.cost_get_dashboard`) and `record_provider_usage`
    // share one JSONL-backed store. Idempotent.
    crate::openhuman::cost::init_global(cfg.cost.clone(), &workspace_dir);

    // --- x402 payment ledger ---
    // Initializes the JSONL-backed spending ledger for machine-payable API
    // payments (x402 protocol). Budget defaults can be overridden via
    // the `openhuman.x402_update_budget` RPC. Gated on the Web3 domain (#4808
    // review): under `harness()`/`none()` the x402 controllers are absent, so
    // their ledger must not initialize either.
    if domains.allows(crate::core::all::DomainGroup::Web3) {
        let x402_session = format!("x402-{}", uuid::Uuid::new_v4());
        crate::openhuman::x402::init_ledger(&workspace_dir, &x402_session);
    } else {
        log::debug!("[boot] x402 payment ledger SKIPPED — Web3 domain disabled");
    }

    // --- Sub-agent definition registry bootstrap ---
    // Loads built-in archetype definitions plus any custom TOML files
    // under `<workspace>/agents/*.toml`. Idempotent — safe to call
    // multiple times. Uses the per-user scoped workspace_dir.
    if let Err(err) =
        crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(&workspace_dir)
    {
        log::warn!(
            "[runtime] AgentDefinitionRegistry::init_global failed: {err} — \
             spawn_subagent will be unavailable until restart"
        );
    }

    // --- Agent sandbox + projects dirs ---
    // Create the action sandbox + default projects home and register the
    // projects dir as a ReadWrite trusted root BEFORE building the live policy
    // below (so the trusted root is reflected in `from_config`). This is the
    // always-run boot for web-chat-only desktop cores; without it a fresh
    // install with no messaging integrations leaves `~/OpenHuman/projects`
    // uncreated and every shell-tool `current_dir` fails with ERROR_DIRECTORY
    // (os error 267) on Windows / ENOENT on Unix (#3353, RC-A). Idempotent — a
    // later `start_channels` calls the same helper.
    crate::openhuman::config::ensure_agent_dirs(&mut cfg).await;

    // --- Live SecurityPolicy ---
    // Install the process-global live policy on the always-run serve boot, not
    // only inside `start_channels` (which is skipped for web-chat-only cores
    // with no messaging integrations). Without this, `live_policy::current()`
    // would be empty on those cores, so the ApprovalGate's `auto_approve`
    // allowlist and `config.update_autonomy_settings` reloads (`reload_from`)
    // would be inert until a session with integrations starts. `from_config`
    // injects the default projects root, so this matches what `start_channels`
    // installs; idempotent — a later `start_channels` re-installs an equivalent
    // policy.
    let action_dir = cfg.action_dir.clone();
    crate::openhuman::security::live_policy::install(
        std::sync::Arc::new(
            crate::openhuman::security::SecurityPolicy::from_config(
                &cfg.autonomy,
                &workspace_dir,
                &action_dir,
            )
            .with_privacy_mode(cfg.privacy.mode),
        ),
        workspace_dir.clone(),
        action_dir,
    );

    // --- Triggered-workflow subscriber ---
    // Install on the always-run serve boot, not only inside `start_channels`
    // (skipped for web-chat-only cores with no messaging integrations, and when
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS=1`). Without this, any workflow
    // declaring `triggers:` was silently ignored on web-chat-only desktop
    // installs. Idempotent — shares a process-global OnceLock with the
    // `start_channels` site so it registers exactly once regardless of which
    // path runs first. (Matching only for now; activation handoff still pending.)
    // Gated on the Skills domain (#4808 review): under `harness()`/`none()` the
    // skills controllers are absent, so their trigger subscriber must not install.
    if domains.allows(crate::core::all::DomainGroup::Skills) {
        crate::openhuman::skills::bus::ensure_triggered_workflow_subscriber(&workspace_dir);
    } else {
        log::debug!("[boot] triggered-workflow subscriber SKIPPED — Skills domain disabled");
    }

    // --- Approval gate (#1339) ---
    // ON by default; opt out with `OPENHUMAN_APPROVAL_GATE=0` (or `false`).
    // Prompt-class `external_effect()` tool calls route through
    // `ApprovalGate::intercept` and park until the UI dispatches
    // `approval_decide` (or the 10-minute TTL elapses → deny). Safe to default
    // on now that the release surface exists (ApprovalRequestCard + the Agent
    // OS access panel) AND only *interactive chat* turns park — background /
    // triage / cron turns carry no chat context and pass straight through, so
    // autonomous automation is never blocked.
    //
    // Host-aware override evaluation: under the Tauri desktop shell the env
    // override is treated as advisory only — the gate ALWAYS installs and a
    // `DomainEvent::ApprovalGateOverrideIgnored` is published so the UI can
    // surface a one-shot banner explaining the override was rejected. Under
    // standalone CLI / Docker (env-as-config is the operator's chosen
    // surface) the override is honored, but a `DomainEvent::ApprovalGateDisabled`
    // is still published so any connected dashboard / log shipper can
    // surface the elevated-privilege state.
    let env_override_requested = std::env::var("OPENHUMAN_APPROVAL_GATE")
        .map(|v| {
            let t = v.trim();
            t == "0" || t.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false);
    let decision =
        crate::core::types::approval_gate_boot_decision(host_kind, env_override_requested);
    // Record the boot decision before publishing the warning event so the
    // first poll of `approval_get_gate_state` after boot reflects the same
    // host-aware verdict the event itself describes — no race.
    crate::openhuman::approval::gate::record_boot_state(
        crate::openhuman::approval::gate::ApprovalGateBootState {
            installed: decision.install_gate,
            disabled_by_env: decision.gate_disabled_by_override,
            override_ignored: decision.override_ignored,
            host: match host_kind {
                crate::core::types::HostKind::TauriShell => "tauri-shell",
                crate::core::types::HostKind::Cli => "cli",
                crate::core::types::HostKind::Docker => "docker",
            },
        },
    );
    if decision.override_ignored {
        log::warn!(
            "[runtime] OPENHUMAN_APPROVAL_GATE=0 IGNORED under desktop shell — \
             gate is always on for the Tauri host (host={})",
            host_kind.tag()
        );
        crate::core::event_bus::publish_global(
            crate::core::event_bus::DomainEvent::ApprovalGateOverrideIgnored {
                host: host_kind.tag().to_string(),
            },
        );
    }
    // Bridge interactive web-surface events to the frontend: ApprovalRequested →
    // `approval_request` AND PlanReviewRequested → `plan_review_request` (both
    // handled by the same subscriber). Registered UNCONDITIONALLY here on the
    // always-run serve boot — the plan-review gate is independent of the approval
    // gate and parks turns even when `OPENHUMAN_APPROVAL_GATE=0`, while
    // `start_channels` is skipped for web-chat-only cores. Without this an
    // unguarded standalone/CLI/Docker core would park a plan review that never
    // reaches the UI and dies at the gate TTL. Idempotent (Once-guarded).
    crate::openhuman::channels::providers::web::register_approval_surface_subscriber();

    if decision.install_gate {
        // Per-launch correlation token for the approval gate. This is
        // a fresh UUID every boot — it is NOT derived from the
        // JSON-RPC bearer (`OPENHUMAN_CORE_TOKEN` / the in-memory
        // auth subsystem) and carries no credential material, so it
        // is safe to log, persist, and surface in audit events.
        // `approval_list_pending` is session-agnostic so pending rows
        // from prior launches remain visible after restart; only the
        // per-session audit grouping changes across launches.
        let session_id = format!("session-{}", uuid::Uuid::new_v4());
        let _ =
            crate::openhuman::approval::ApprovalGate::init_global(cfg.clone(), session_id.clone());
        log::info!(
            "[runtime] approval gate installed (on by default; set OPENHUMAN_APPROVAL_GATE=0 to disable, session_id={session_id}) — \
             Prompt-class external-effect tool calls park for approval in interactive chat turns"
        );
        // (The approval/plan-review surface bridge is registered unconditionally
        // above — it must run even when this gate-install branch is skipped.)
        crate::openhuman::channels::providers::web::register_artifact_surface_subscriber();
    } else {
        log::error!(
            "[runtime] approval gate DISABLED (OPENHUMAN_APPROVAL_GATE=0 honored on host={}) — \
             Prompt-class external-effect tool calls run unprompted",
            host_kind.tag()
        );
        crate::core::event_bus::publish_global(
            crate::core::event_bus::DomainEvent::ApprovalGateDisabled {
                host: host_kind.tag().to_string(),
                reason: "env-override".to_string(),
            },
        );
    }
    // Artifact surface bridges DomainEvent::ArtifactReady/Failed onto the web
    // channel ("Files in this chat" panel + ArtifactCard updates). This is
    // independent of the approval-gate config — keep it outside the
    // `if approval_gate` block so artifact events still publish when the user
    // sets OPENHUMAN_APPROVAL_GATE=0 (CR #3328947323 on PR #3026). Idempotent
    // (OnceLock-guarded inside register_artifact_surface_subscriber).
    crate::openhuman::channels::providers::web::register_artifact_surface_subscriber();

    // --- Workspace migrations --------------------------------------------
    crate::openhuman::startup::run_workspace_migrations(&workspace_dir);

    // --- Socket manager bootstrap ---
    let socket_mgr = Arc::new(SocketManager::new());
    set_global_socket_manager(socket_mgr.clone());
    log::info!("[socket] SocketManager initialized and registered globally");
}

/// Starts selected background jobs after the runtime has entered `serve()`.
///
/// This deliberately sits outside [`bootstrap_core_runtime`]: embedders may call
/// `CoreBuilder::build()` only to use in-process RPC, and a failed listener bind
/// must not leave pollers, one-shot jobs, MCP processes, or socket reconnect work
/// running without a live runtime.
pub fn start_core_runtime_services(
    services: crate::core::runtime::ServiceSet,
    config: Option<&crate::openhuman::config::Config>,
) {
    let Some(cfg) = config else {
        log::error!(
            "[runtime] Config unavailable for runtime service startup; selected services skipped"
        );
        return;
    };

    // Long-lived bootstrap loops selected by ServiceSet.
    crate::core::runtime::services::start_bootstrap_jobs(services, cfg);

    // One-time first-run initialization (managed Python runtime, spaCy model,
    // managed Node runtime). Spawned AFTER subscribers are live but does NOT
    // block the ready signal — the core becomes RPC-ready immediately and the
    // frontend watches per-step progress via `openhuman.harness_init_status`.
    // On a warm host every step's `is_done` probe passes and this settles
    // instantly. See `crate::openhuman::harness_init`.
    crate::core::runtime::services::start_boot_once_jobs(services, cfg);

    match crate::openhuman::socket::global_socket_manager() {
        Some(socket_mgr) => {
            crate::core::runtime::services::spawn_socket_auto_connect(services, socket_mgr.clone());
        }
        None => {
            log::warn!(
                "[socket] SocketManager unavailable during runtime service startup; auto-connect skipped"
            );
        }
    }
}

/// JSON-serializable wrapper for the entire RPC schema dump.
#[derive(Serialize)]
struct HttpSchemaDump {
    /// List of all available RPC methods and their schemas.
    methods: Vec<HttpMethodSchema>,
}

/// JSON-serializable schema for a single RPC method.
#[derive(Serialize)]
struct HttpMethodSchema {
    /// Fully qualified JSON-RPC method name.
    method: String,
    /// Namespace of the function.
    namespace: String,
    /// Function name within the namespace.
    function: String,
    /// Human-readable description of what the method does.
    description: String,
    /// List of input parameters.
    inputs: Vec<crate::core::FieldSchema>,
    /// List of output fields.
    outputs: Vec<crate::core::FieldSchema>,
}

/// Aggregates schemas from all registered controllers into a single dump.
///
/// Also includes built-in core methods like `core.ping` and `core.version`.
fn build_http_schema_dump() -> HttpSchemaDump {
    let mut methods: Vec<HttpMethodSchema> = all::all_http_method_schemas()
        .into_iter()
        .map(|method| HttpMethodSchema {
            method: method.method,
            namespace: method.namespace.to_string(),
            function: method.function.to_string(),
            description: method.description.to_string(),
            inputs: method.inputs,
            outputs: method.outputs,
        })
        .collect();

    // Sort methods alphabetically for consistent output.
    methods.sort_by(|a, b| a.method.cmp(&b.method));

    HttpSchemaDump { methods }
}

#[cfg(test)]
#[path = "jsonrpc_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "jsonrpc_cors_tests.rs"]
mod cors_tests;
