//! JSON-RPC 2.0 server implementation for OpenHuman.
//!
//! This module provides:
//! - An Axum-based HTTP server for handling JSON-RPC requests.
//! - Method dispatching to registered controllers.
//! - SSE (Server-Sent Events) for real-time event streaming.
//! - Helper routes for health checks, schema discovery, and Telegram authentication.

use std::sync::Arc;

use axum::extract::{Query, State, WebSocketUpgrade};
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
            let (display_message, error_data, expected_user_state) = match structured {
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
            } else if is_param_validation_error(&display_message) {
                tracing::info!(
                    method = %method,
                    elapsed_ms = ms as u64,
                    "[rpc] param-validation error (message redacted; skip-report)"
                );
            } else if is_session_expired_error(&display_message) {
                tracing::info!("[rpc] {} -> err ({}ms): {}", method, ms, display_message);
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
    match crate::openhuman::credentials::ops::store_session(&config, &jwt_token, None, None).await {
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

    match crate::openhuman::credentials::ops::store_session(&config, &jwt_token, None, None).await {
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

/// WebSocket upgrade handler for streaming voice dictation.
async fn dictation_ws_handler(ws: WebSocketUpgrade) -> Response {
    log::info!("[ws] dictation WebSocket upgrade requested");
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
    let router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/schema", get(schema_handler))
        .route("/events", get(events_handler))
        .route("/events/webhooks", get(webhook_events_handler))
        .route("/events/domain", get(domain_events_handler))
        .route("/rpc", post(rpc_handler))
        .route("/ws/dictation", get(dictation_ws_handler))
        .route("/auth", get(desktop_auth_handler))
        .route("/auth/telegram", get(telegram_auth_handler))
        // OpenAI-compatible inference endpoint (/v1/chat/completions, /v1/models)
        .nest("/v1", crate::openhuman::inference::http::router())
        .fallback(not_found_handler)
        .layer(middleware::from_fn(http_request_log_middleware))
        .layer(middleware::from_fn(crate::core::auth::rpc_auth_middleware))
        .layer(middleware::from_fn(cors_middleware))
        .with_state(AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        });

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
async fn health_handler() -> impl IntoResponse {
    let snapshot = crate::openhuman::health::snapshot();
    let unhealthy: Vec<&str> = snapshot
        .components
        .iter()
        .filter_map(|(name, c)| {
            if c.status == "ok" || c.status == "starting" {
                None
            } else {
                Some(name.as_str())
            }
        })
        .collect();
    let is_ok = unhealthy.is_empty();

    let status = if is_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    tracing::debug!(
        "[health] status={} components={} unhealthy={:?}",
        status.as_u16(),
        snapshot.components.len(),
        unhealthy
    );

    (status, Json(snapshot))
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
fn core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

/// Resolves the bind address host for the core server from environment variables or defaults.
fn core_host() -> String {
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
    // Ensure all controllers are registered before starting.
    let _ = all::all_registered_controllers();

    // Ensure the master encryption key is loaded from keychain before any
    // config or credential operation that needs to decrypt secrets. This is
    // a no-op if already called (e.g. from run_core_from_args for CLI).
    crate::openhuman::keyring::init_master_key();

    // Initialize the per-process RPC bearer token.
    //
    // Preferred path (in-process core spawned by the Tauri shell): the caller
    // passes the bearer it already holds in `CoreProcessHandle.rpc_token` as
    // `rpc_token: Some(_)`. The token is seeded directly into the auth
    // subsystem without ever crossing `OPENHUMAN_CORE_TOKEN` on the process
    // environment — closing the same-UID readback channel (sysctl
    // KERN_PROCARGS2 / ps eww on macOS, /proc/<pid>/environ on Linux).
    //
    // Fallback (standalone CLI / docker / cloud `openhuman core run`):
    // `rpc_token: None` lets `init_rpc_token` read `OPENHUMAN_CORE_TOKEN`
    // from the environment when present (env-as-config — legit operator
    // surface), or generate a fresh token and write `{workspace_dir}/core.token`
    // (0o600 on Unix) so CLI callers can authenticate.
    if let Some(token) = rpc_token.as_deref() {
        crate::core::auth::init_rpc_token_with_value(token)?;
    } else {
        let token_dir =
            crate::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|_| {
                dirs::home_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join(".openhuman")
            });
        crate::core::auth::init_rpc_token(&token_dir)?;
    }

    // Initialize the global MemoryClient so composio providers
    // (gmail/slack/notion) can persist their sync_state via kv_get/kv_set,
    // and so any subsystem that calls `memory::global::client_if_ready()`
    // gets a live handle. Without this, every periodic sync bails with
    // "[composio:gmail] memory client not ready".
    {
        // A `Config::load_or_init` failure here is operator-visible and
        // serious (corrupt toml, bad permissions, missing/unwritable
        // OPENHUMAN_WORKSPACE — common on headless/containerised deploys
        // with no writable $HOME). Previously we fell back to
        // `Config::default()` and initialised the memory + whatsapp_data
        // stores against the *wrong* workspace dir, silently causing chunk
        // loss / cross-workspace bleed-over while the app looked healthy
        // (Sentry OPENHUMAN-CORE-48). Instead: skip the workspace-bound
        // init entirely so memory stays explicitly *uninitialised* —
        // callers then get a clear "memory client not ready" error rather
        // than reading/writing the wrong workspace. The server still comes
        // up; the operator sees the loud error and fixes their config or
        // sets OPENHUMAN_WORKSPACE to a writable path, then restarts.
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(cfg) => {
                match crate::openhuman::memory::global::init(cfg.workspace_dir.clone()) {
                    Ok(_) => log::info!(
                        "[boot] memory::global initialized (workspace={})",
                        cfg.workspace_dir.display()
                    ),
                    Err(e) => log::warn!("[boot] memory::global init failed: {e}"),
                }
                // Initialize the WhatsApp data store so scanner ingest calls
                // can write data without requiring a lazy-init fallback.
                match crate::openhuman::whatsapp_data::global::init(cfg.workspace_dir.clone()) {
                    Ok(_) => log::info!(
                        "[boot] whatsapp_data::global initialized (workspace={})",
                        cfg.workspace_dir.display()
                    ),
                    Err(e) => log::warn!("[boot] whatsapp_data::global init failed: {e}"),
                }
                // Seed bundled default skills into <workspace>/skills/ so they
                // ship with the system — discoverable (skills_list) and runnable
                // — without a manual drop. Idempotent; never clobbers user edits.
                crate::openhuman::skills::registry::seed_default_skills(&cfg.workspace_dir);
            }
            Err(e) => {
                log::error!(
                    "[boot] memory::global + whatsapp_data init SKIPPED — \
                     Config::load_or_init failed ({e:#}). Memory persistence is \
                     DISABLED for this run; no silent fallback to the default \
                     workspace (which would cause chunk loss / cross-workspace \
                     bleed-over). Fix config.toml or set OPENHUMAN_WORKSPACE to a \
                     writable path, then restart."
                );
            }
        }
    }

    let (resolved_port, port_source) = match port {
        Some(p) => (p, "CLI --port"),
        None => (
            core_port(),
            if std::env::var("OPENHUMAN_CORE_PORT").is_ok() {
                "env OPENHUMAN_CORE_PORT"
            } else {
                "default"
            },
        ),
    };
    let (resolved_host, host_source) = match host {
        Some(h) => (h.to_string(), "CLI --host"),
        None => (
            core_host(),
            if std::env::var("OPENHUMAN_CORE_HOST")
                .ok()
                .filter(|s| !s.is_empty())
                .is_some()
            {
                "env OPENHUMAN_CORE_HOST"
            } else {
                "default"
            },
        ),
    };

    log::debug!(
        "[core] Bind resolution: host={resolved_host} (from {host_source}), port={resolved_port} (from {port_source})"
    );

    // Safety check: refuse to bind on a non-loopback address without an
    // explicit RPC token. Without this, the entire RPC surface (tool
    // execution, file access, credentials) is unauthenticated and reachable
    // from the network. See: https://github.com/tinyhumansai/openhuman/issues/1919
    //
    // "Explicit token" means any of:
    //   - An in-memory bearer supplied by the embedded caller via the
    //     `rpc_token` parameter (the Tauri shell hands its
    //     `CoreProcessHandle.rpc_token` in this way — see
    //     `init_rpc_token_with_value`). This never lands on the process env.
    //   - `OPENHUMAN_CORE_TOKEN` set in the process environment (operator
    //     config for standalone CLI / Docker / cloud).
    //
    // Checking only the env var would emit a false security warning whenever
    // an embedded caller binds on a non-loopback host with an in-memory
    // bearer — the server is already protected in that case.
    if crate::openhuman::security::pairing::is_public_bind(&resolved_host) {
        let has_in_memory_token = rpc_token
            .as_deref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        let has_env_token = std::env::var(crate::core::auth::CORE_TOKEN_ENV_VAR)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .is_some();
        // Auth subsystem was already initialised above; fall back to the live
        // token if neither input matched but somehow a token is seeded (e.g.
        // a future caller route that doesn't thread the value through here).
        let has_initialized_token = crate::core::auth::get_rpc_token()
            .map(|t| !t.trim().is_empty())
            .unwrap_or(false);
        let has_explicit_token = has_in_memory_token || has_env_token || has_initialized_token;
        if !has_explicit_token {
            log::error!(
                "[core] ⚠️  SECURITY WARNING: Binding on public address {resolved_host} without \
                 an explicit OPENHUMAN_CORE_TOKEN. The RPC server will auto-generate a token, \
                 but external clients will not know it. Set OPENHUMAN_CORE_TOKEN in your \
                 .env file to secure the RPC endpoint."
            );
            eprintln!(
                "\n\x1b[1;31m[SECURITY]\x1b[0m Binding on {resolved_host} without OPENHUMAN_CORE_TOKEN.\n\
                 Set OPENHUMAN_CORE_TOKEN in .env to secure the RPC endpoint.\n\
                 Without it, the auto-generated token is written to {{workspace}}/core.token\n\
                 but remote clients will not be able to authenticate.\n"
            );
        }
    }

    let preferred_port = resolved_port;
    let host = resolved_host;
    let pick = crate::openhuman::connectivity::rpc::pick_listen_port_for_host(
        host.as_str(),
        preferred_port,
    )
    .await
    .map_err(|err| {
        log::error!("[core] Failed to bind to {host}:{preferred_port}: {err}");
        anyhow::Error::new(err)
    })?;
    let listen_port = pick.port;
    let bind_addr = format!("{host}:{listen_port}");
    let listener = pick.listener;

    // Synchronize OPENHUMAN_CORE_RPC_URL with the actual bound port so
    // connectivity::rpc::resolve_listen_port() (used by openhuman.connectivity_diag)
    // reports the live listener instead of the originally-requested port when
    // fallback engaged. Embedded path also calls this via apply_embedded_ready_signal,
    // but the standalone CLI never did before — leaving diag stale on fallback.
    //
    // SAFETY: set_var is process-global; this runs once during bind and the
    // standalone CLI doesn't share its env with concurrent test threads.
    unsafe {
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", format!("http://{bind_addr}/rpc"));
    }

    let app = build_core_http_router(socketio_enabled);

    // --- Core runtime bootstrap --------------------------------------------
    bootstrap_core_runtime(embedded_core).await;

    log::info!(
        "[core] OpenHuman core is ready — listening on http://{bind_addr} (version {})",
        env!("CARGO_PKG_VERSION")
    );
    log::info!("[rpc:http] JSON-RPC — POST http://{bind_addr}/rpc (JSON-RPC 2.0)");
    if socketio_enabled {
        log::info!("[rpc:socketio] Socket.IO — ws://{bind_addr}/socket.io/ (same HTTP server)");
    } else {
        log::info!("[rpc:socketio] disabled (--jsonrpc-only)");
    }

    if let Some(tx) = ready_tx {
        let _ = tx.send(EmbeddedReadySignal {
            port: listen_port,
            fallback_from: pick.fallback_from,
        });
    }

    // Background bootstrap for services — gated on login state.
    //
    // Heavy services (local AI, voice, screen intelligence, autocomplete)
    // are only started when a user is logged in. If no user session exists
    // on disk, startup is deferred until the login handler in
    // `credentials::ops::store_session()` triggers it.
    tokio::spawn(async move {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                if embedded_core {
                    log::debug!("[core] embedded core startup");
                } else {
                    log::debug!("[core] desktop core startup");
                }

                // Register autocomplete shutdown hook so the engine (and its
                // Swift overlay helper) are stopped cleanly on process exit.
                // This is unconditional — the hook should fire regardless of
                // whether the user is currently logged in.
                crate::core::shutdown::register(|| async {
                    let engine = crate::openhuman::autocomplete::global_engine();
                    let status = engine.status().await;
                    if status.running {
                        log::info!(
                            "[core] stopping autocomplete engine (phase={})",
                            status.phase
                        );
                        engine.stop(None).await;
                        log::info!("[core] autocomplete engine stopped");
                    }
                });

                // Check if a user is already logged in from a previous session.
                let already_logged_in = crate::openhuman::config::default_root_openhuman_dir()
                    .ok()
                    .and_then(|root| crate::openhuman::config::read_active_user_id(&root))
                    .is_some();

                if already_logged_in {
                    // User has an active session — start all services now.
                    log::info!("[services] existing session found, starting services");
                    crate::openhuman::credentials::ops::start_login_gated_services(&config).await;

                    // Subconscious engine + heartbeat.
                    if !config.heartbeat.enabled {
                        log::info!("[subconscious] disabled by config (heartbeat.enabled = false)");
                    } else {
                        match crate::openhuman::subconscious::global::bootstrap_after_login().await
                        {
                            Ok(()) => log::info!(
                                "[subconscious] bootstrapped on startup (existing session)"
                            ),
                            Err(e) => log::warn!("[subconscious] startup bootstrap failed: {e}"),
                        }
                    }
                } else {
                    log::info!(
                        "[services] no active session — deferring service startup until login"
                    );
                }
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping service startup: {err}");
            }
        }
    });

    // Periodic self-update checker (default: every 1 hour).
    tokio::spawn(async {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                crate::openhuman::update::scheduler::run(config.update).await;
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping update scheduler: {err}");
            }
        }
    });

    // Cron scheduler — polls due_jobs() every ~5s and executes them automatically.
    tokio::spawn(async {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                if !config.cron.enabled {
                    log::info!("[cron] scheduler disabled via config; skipping");
                    return;
                }
                log::info!("[cron] spawning scheduler polling loop");
                if let Err(e) = crate::openhuman::cron::scheduler::run(config).await {
                    log::error!("[cron] scheduler loop ended with error: {e}");
                }
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping cron scheduler: {err}");
            }
        }
    });

    // Realtime channel listeners (Telegram getUpdates, Discord gateway, etc.) live in
    // `start_channels`. Without this task, `openhuman run` would only expose RPC while
    // inbound bot messages are never polled.
    if std::env::var("OPENHUMAN_DISABLE_CHANNEL_LISTENERS")
        .ok()
        .filter(|s| s == "1" || s.eq_ignore_ascii_case("true"))
        .is_none()
    {
        tokio::spawn(async move {
            let config = match crate::openhuman::config::Config::load_or_init().await {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("[channels] could not load config for listeners: {e}");
                    return;
                }
            };
            if !config.channels_config.has_listening_integrations() {
                log::debug!(
                    "[channels] no channel integrations configured; not spawning listeners"
                );
                return;
            }
            log::info!("[channels] spawning in-process realtime listeners (Telegram, Discord, …)");
            if let Err(e) = crate::openhuman::channels::start_channels(config).await {
                log::error!("[channels] start_channels ended with error: {e}");
            }
        });
    } else {
        log::info!("[channels] OPENHUMAN_DISABLE_CHANNEL_LISTENERS set — skipping start_channels");
    }

    if let Some(shutdown_token) = shutdown_token {
        log::info!("[core] embedded server waiting on cancellation token for graceful shutdown");
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_token.cancelled().await;
            })
            .await?;
    } else {
        axum::serve(listener, app)
            .with_graceful_shutdown(crate::core::shutdown::signal())
            .await?;
    }

    // Server has stopped accepting and in-flight requests drained.
    // Kill any `ollama serve` openhuman itself spawned (no-op when the
    // daemon was externally managed) and clear the spawn marker so the
    // next launch doesn't try to reclaim a daemon that's already dead.
    // Bounded so a wedged Ollama can't hold up app shutdown.
    if let Some(svc) = crate::openhuman::inference::local::try_global() {
        let cfg = crate::openhuman::config::Config::load_or_init()
            .await
            .unwrap_or_default();
        log::info!("[core] shutdown: cleaning up openhuman-owned ollama if any");
        let shutdown_fut = svc.shutdown_owned_ollama(&cfg);
        if tokio::time::timeout(std::time::Duration::from_secs(2), shutdown_fut)
            .await
            .is_err()
        {
            log::warn!("[core] shutdown: ollama cleanup exceeded 2s budget; proceeding with exit");
        }
    }

    Ok(())
}

/// Registers all long-lived domain event-bus subscribers exactly once.
///
/// Guarded by `std::sync::Once` so repeated calls to `bootstrap_core_runtime`
/// are safe and idempotent.
fn register_domain_subscribers(
    workspace_dir: std::path::PathBuf,
    config: crate::openhuman::config::Config,
    embedded_core: bool,
) {
    use std::sync::{Arc, Once};

    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        // Leak the SubscriptionHandle so the background tasks live for the
        // entire process — SubscriptionHandle::drop aborts the task.
        if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
            crate::openhuman::webhooks::bus::WebhookRequestSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!("[event_bus] failed to register webhook subscriber — bus not initialized");
        }

        if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
            crate::openhuman::channels::bus::ChannelInboundSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!("[event_bus] failed to register channel subscriber — bus not initialized");
        }

        crate::openhuman::health::bus::register_health_subscriber();
        crate::openhuman::notifications::register_notification_bridge_subscriber();
        crate::openhuman::memory_conversations::register_conversation_persistence_subscriber(
            workspace_dir.clone(),
        );
        crate::openhuman::memory::sync::register_sync_stage_bridge();
        if let Err(error) = crate::openhuman::composio::init_composio_trigger_history(
            workspace_dir.clone(),
        ) {
            log::warn!("[composio][history] failed to initialize trigger archive: {error}");
        }
        crate::openhuman::composio::register_composio_trigger_subscriber();
        crate::openhuman::composio::start_periodic_sync();
        // Initialise the scheduler gate before any background AI workers
        // start so they observe a real policy on their first iteration
        // (otherwise they fall back to `Policy::Normal` and miss the
        // initial throttle decision on battery-powered hosts).
        crate::openhuman::scheduler_gate::init_global(&config);

        // Seed the scheduler-gate signed-out override from the on-disk
        // session. Without this, a sidecar that boots with no stored JWT
        // would happily spin up cron / channel loops and fire LLM requests
        // that all 401 immediately.
        match crate::api::jwt::get_session_token(&config) {
            Ok(Some(_)) => {
                crate::openhuman::scheduler_gate::set_signed_out(false);
            }
            Ok(None) => {
                log::info!(
                    "[auth] no session token at startup — scheduler gate set to signed_out"
                );
                crate::openhuman::scheduler_gate::set_signed_out(true);
            }
            Err(err) => {
                log::warn!(
                    "[auth] failed to read session token at startup ({err}) — assuming signed_out"
                );
                crate::openhuman::scheduler_gate::set_signed_out(true);
            }
        }

        // Register the SessionExpired handler before any subscribers that
        // might publish 401-derived events, so the very first 401 is
        // routed through `clear_session` + the scheduler-gate override.
        if let Some(handle) = crate::core::event_bus::subscribe_global(Arc::new(
            crate::openhuman::credentials::bus::SessionExpiredSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!(
                "[event_bus] failed to register SessionExpired subscriber — bus not initialized"
            );
        }

        crate::openhuman::memory_queue::start(config.clone());

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

        // Proactive message subscriber (web-only in the desktop runtime —
        // no external channel instances are registered here). Uses a
        // Once-guarded registrar so domain-level startup can't duplicate it.
        crate::openhuman::channels::proactive::register_web_only_proactive_subscriber();

        // Device tunnel subscriber: handles tunnel:frame handshakes, peer-status
        // events, and register acks. Must be registered before any tunnel:frame
        // events can arrive.
        crate::openhuman::devices::bus::register_device_tunnel_subscriber();

        // Native request handlers — typed in-process request/response.
        // The agent `agent.run_turn` handler is what channel dispatch
        // calls instead of importing `run_tool_call_loop` directly.
        crate::openhuman::agent::bus::register_agent_handlers();

        log::info!(
            "[event_bus] domain subscribers registered (webhook, channel, health, conversation, composio, restart, proactive, agent, session_expired)"
        );
    });
}

/// Initializes long-lived socket/event-bus infrastructure.
pub async fn bootstrap_core_runtime(embedded_core: bool) {
    use crate::openhuman::socket::{set_global_socket_manager, SocketManager};
    use std::sync::Arc;
    let cfg = match crate::openhuman::config::Config::load_or_init().await {
        Ok(cfg) => cfg,
        Err(e) => {
            log::error!("[runtime] Failed to load config for socket manager: {e}");
            return;
        }
    };
    let workspace_dir = cfg.workspace_dir.clone();

    // --- Event bus bootstrap ---
    // Ensure the global event bus is initialized (no-op if already done by start_channels).
    crate::core::event_bus::init_global(crate::core::event_bus::DEFAULT_CAPACITY);
    // Register domain subscribers for cross-module event handling.
    // Uses a Once guard so repeated calls to bootstrap_core_runtime()
    // cannot double-subscribe.
    register_domain_subscribers(workspace_dir.clone(), cfg.clone(), embedded_core);

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

    // --- Cost dashboard tracker ---
    // Activates the previously-dormant CostTracker so the dashboard RPC
    // surface (`openhuman.cost_get_dashboard`) and `record_provider_usage`
    // share one JSONL-backed store. Idempotent.
    crate::openhuman::cost::init_global(cfg.cost.clone(), &workspace_dir);

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
    crate::openhuman::security::live_policy::install(
        std::sync::Arc::new(crate::openhuman::security::SecurityPolicy::from_config(
            &cfg.autonomy,
            &workspace_dir,
        )),
        workspace_dir.clone(),
    );

    // --- Approval gate (#1339) ---
    // ON by default; opt out with `OPENHUMAN_APPROVAL_GATE=0` (or `false`).
    // Prompt-class `external_effect()` tool calls route through
    // `ApprovalGate::intercept` and park until the UI dispatches
    // `approval_decide` (or the 10-minute TTL elapses → deny). Safe to default
    // on now that the release surface exists (ApprovalRequestCard + the Agent
    // OS access panel) AND only *interactive chat* turns park — background /
    // triage / cron turns carry no chat context and pass straight through, so
    // autonomous automation is never blocked.
    if std::env::var("OPENHUMAN_APPROVAL_GATE")
        .map(|v| {
            let t = v.trim();
            !(t == "0" || t.eq_ignore_ascii_case("false"))
        })
        .unwrap_or(true)
    {
        // Read the active bearer from the in-memory auth subsystem instead of
        // OPENHUMAN_CORE_TOKEN — same value (init_rpc_token / init_rpc_token_with_value
        // both populate the OnceLock that get_rpc_token reads), no env crossing.
        // Init ordering: run_server_inner seeds the auth subsystem above
        // (line ~1340) before bootstrap_core_runtime runs, so the bearer is
        // always present here when supplied.
        let (session_id, ephemeral) = match crate::core::auth::get_rpc_token() {
            Some(token) => (token.to_string(), false),
            None => (format!("session-{}", uuid::Uuid::new_v4()), true),
        };
        if ephemeral {
            log::debug!(
                "[runtime] auth bearer uninitialized; generated ephemeral session_id={session_id} \
                 for approval gate — `approval_list_pending` is session-agnostic so pending rows \
                 from prior launches will still be visible, but per-session audit grouping will not \
                 correlate across restarts"
            );
        }
        let _ =
            crate::openhuman::approval::ApprovalGate::init_global(cfg.clone(), session_id.clone());
        // Never log a token-derived session_id: when OPENHUMAN_CORE_TOKEN is set,
        // session_id IS that secret. Only the generated ephemeral UUID is safe to
        // print.
        let session_label = if ephemeral {
            session_id.as_str()
        } else {
            "<redacted>"
        };
        log::info!(
            "[runtime] approval gate installed (on by default; set OPENHUMAN_APPROVAL_GATE=0 to disable, session_id={session_label}) — \
             Prompt-class external-effect tool calls park for approval in interactive chat turns"
        );
        // Bridge ApprovalRequested → `approval_request` web socket event. This MUST
        // be registered here on the always-run serve boot, not only inside
        // `start_channels` — that path is skipped when no messaging integrations
        // (Telegram/Discord/…) are configured, which is the common web-chat-only
        // case. Without this, the gate parks and publishes but nothing reaches the
        // frontend → every prompt dies at the TTL. Idempotent (Once-guarded).
        crate::openhuman::channels::providers::web::register_approval_surface_subscriber();
    } else {
        log::info!(
            "[runtime] approval gate disabled (OPENHUMAN_APPROVAL_GATE=0) — \
             Prompt-class external-effect tool calls run unprompted"
        );
    }

    // --- Workspace migrations --------------------------------------------
    crate::openhuman::startup::run_workspace_migrations(&workspace_dir);

    // --- MCP registry boot-spawn -----------------------------------------
    // Bring up every locally-installed MCP server's stdio subprocess so its
    // tools are available to the agent as soon as the core is ready.
    // Errors are logged per-server and never block boot. Runs as a
    // background task so a slow npx install can't gate startup.
    {
        let cfg = cfg.clone();
        tokio::spawn(async move {
            crate::openhuman::mcp_registry::boot::spawn_installed_servers(&cfg).await;
        });
    }

    // --- Socket manager bootstrap ---
    let socket_mgr = Arc::new(SocketManager::new());
    set_global_socket_manager(socket_mgr.clone());
    log::info!("[socket] SocketManager initialized and registered globally");

    // Auto-connect socket to backend if a session token is already stored.
    // This runs in the background so it doesn't block server startup.
    tokio::spawn(async move {
        log::info!("[socket] Checking for stored session to auto-connect...");
        let config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                log::debug!("[socket] Config not available for auto-connect: {e}");
                return;
            }
        };
        let api_url = crate::api::config::effective_backend_api_url(&config.api_url);
        let token = match crate::api::jwt::get_session_token(&config) {
            Ok(Some(t)) => t,
            Ok(None) => {
                log::info!("[socket] No session token stored — skipping auto-connect (will connect after login)");
                return;
            }
            Err(e) => {
                log::warn!("[socket] Failed to read session token: {e}");
                return;
            }
        };
        log::info!(
            "[socket] Session token found — auto-connecting to {}",
            api_url
        );
        if let Err(e) = socket_mgr.connect(&api_url, &token).await {
            log::error!("[socket] Auto-connect failed: {e}");
        } else {
            log::info!("[socket] Auto-connect initiated successfully");
        }
    });
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
