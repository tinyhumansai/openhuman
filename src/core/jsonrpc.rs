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

use crate::core::all;
use crate::core::types::{AppState, RpcError, RpcFailure, RpcRequest, RpcSuccess};

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
        Err(message) => {
            tracing::info!("[rpc] {} -> err ({}ms): {}", method, ms, message);
            (
                StatusCode::OK,
                Json(RpcFailure {
                    jsonrpc: "2.0",
                    id,
                    error: RpcError {
                        code: -32000,
                        message,
                        data: None,
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
/// automatic session management logic. If a call fails with a 401 Unauthorized
/// error from the backend, it will automatically clear the local session.
///
/// # Arguments
///
/// * `state` - The application state.
/// * `method` - The name of the method to invoke.
/// * `params` - The JSON parameters for the method.
pub async fn invoke_method(state: AppState, method: &str, params: Value) -> Result<Value, String> {
    let result = invoke_method_inner(state, method, params).await;

    // Session auto-cleanup: If the backend says we're unauthorized,
    // we should reflect that locally by clearing the stored token.
    if let Err(ref msg) = result {
        if is_session_expired_error(msg) {
            log::warn!(
                "[jsonrpc] backend returned 401 for method '{}' — clearing stored session",
                method
            );
            if let Ok(config) = crate::openhuman::config::rpc::load_config_with_timeout().await {
                let _ = crate::openhuman::credentials::rpc::clear_session(&config).await;
            }
        }
    }

    result
}

/// Helper to determine if an error message indicates an expired or invalid session.
fn is_session_expired_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    (lower.contains("401") && lower.contains("unauthorized"))
        || lower.contains("invalid token")
        || msg.contains("SESSION_EXPIRED")
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
        let params_obj = params_to_object(params)?;
        // Validate inputs against the schema before calling the handler.
        all::validate_params(&schema, &params_obj)?;
        if let Some(result) = all::try_invoke_registered_rpc(method, params_obj).await {
            return result;
        }
        return Err(format!("registered schema has no handler: {method}"));
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

/// Returns the HTML for a successful connection page.
fn success_html() -> String {
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
        <p>Your Telegram account has been connected to OpenHuman. You can close this tab.</p>
    </div>
</body>
</html>"#
        .to_string()
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

/// Handles the Telegram authentication callback.
///
/// It consumes a one-time token, exchanges it for a JWT from the backend,
/// and stores the session locally.
async fn telegram_auth_handler(Query(query): Query<TelegramAuthQuery>) -> impl IntoResponse {
    let html_response = |status: StatusCode, body: String| -> Response {
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response()
    };

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

    let api_url = crate::api::config::effective_api_url(&config.api_url);

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

    html_response(StatusCode::OK, success_html())
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
        .route("/rpc", post(rpc_handler))
        .route("/ws/dictation", get(dictation_ws_handler))
        .route("/auth/telegram", get(telegram_auth_handler))
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

/// Middleware for handling Cross-Origin Resource Sharing (CORS).
async fn cors_middleware(req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return with_cors_headers(StatusCode::NO_CONTENT.into_response());
    }

    let response = next.run(req).await;
    with_cors_headers(response)
}

/// Injects CORS headers into a response.
fn with_cors_headers(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
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
    (StatusCode::OK, Json(json!({ "ok": true })))
}

/// Handler for the schema discovery endpoint.
async fn schema_handler(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(build_http_schema_dump())).into_response()
}

/// Query parameters for the events SSE endpoint.
#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    /// Unique identifier for the client requesting events.
    client_id: String,
}

/// Handler for the main events SSE endpoint.
///
/// Streams real-time events filtered by `client_id`.
async fn events_handler(
    Query(query): Query<EventsQuery>,
) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>>> {
    let client_id = query.client_id;
    let rx = crate::openhuman::channels::providers::web::subscribe_web_channel_events();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(move |item| {
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
    });

    Sse::new(stream).keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
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

/// Handler for the root endpoint, returning server information and available endpoints.
async fn root_handler() -> impl IntoResponse {
    let api_server = match crate::openhuman::config::Config::load_or_init().await {
        Ok(cfg) => crate::api::config::effective_api_url(&cfg.api_url),
        Err(_) => crate::api::config::effective_api_url(&None),
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
                "events": "/events?client_id=<id>",
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

/// Runs the HTTP/JSON-RPC server.
///
/// This function binds to the specified host and port, initializes the router,
/// bootstraps long-lived runtime infrastructure, and starts serving requests.
pub async fn run_server(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
) -> anyhow::Result<()> {
    run_server_inner(host, port, socketio_enabled, false).await
}

/// Like [`run_server`] but marks the instance as embedded.
pub async fn run_server_embedded(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
) -> anyhow::Result<()> {
    run_server_inner(host, port, socketio_enabled, true).await
}

/// Internal server entrypoint.
async fn run_server_inner(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
    embedded_core: bool,
) -> anyhow::Result<()> {
    // Ensure all controllers are registered before starting.
    let _ = all::all_registered_controllers();

    // Initialize the per-process RPC bearer token.
    // Written to {workspace_dir}/core.token so the Tauri shell can read it.
    let token_dir = crate::openhuman::config::default_root_openhuman_dir().unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join(".openhuman")
    });
    crate::core::auth::init_rpc_token(&token_dir)?;

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

    let port = resolved_port;
    let host = resolved_host;
    let bind_addr = format!("{host}:{port}");
    let listener = tokio::net::TcpListener::bind((host.as_str(), port))
        .await
        .map_err(|e| {
            log::error!("[core] Failed to bind to {bind_addr}: {e}");
            e
        })?;

    let app = build_core_http_router(socketio_enabled);

    // --- Skill runtime bootstrap -------------------------------------------
    bootstrap_skill_runtime().await;

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

    axum::serve(listener, app)
        .with_graceful_shutdown(crate::core::shutdown::signal())
        .await?;
    Ok(())
}

/// Registers all long-lived domain event-bus subscribers exactly once.
///
/// Guarded by `std::sync::Once` so repeated calls to `bootstrap_skill_runtime`
/// are safe and idempotent.
fn register_domain_subscribers(
    workspace_dir: std::path::PathBuf,
    config: crate::openhuman::config::Config,
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
        crate::openhuman::memory::conversations::register_conversation_persistence_subscriber(
            workspace_dir.clone(),
        );
        if let Err(error) = crate::openhuman::composio::init_composio_trigger_history(
            workspace_dir.clone(),
        ) {
            log::warn!("[composio][history] failed to initialize trigger archive: {error}");
        }
        crate::openhuman::composio::register_composio_trigger_subscriber();
        crate::openhuman::composio::start_periodic_sync();
        crate::openhuman::memory::tree::jobs::start(config.clone());

        // Restart requests go through a subscriber so every trigger path shares
        // the same respawn logic.
        crate::openhuman::service::bus::register_restart_subscriber();

        // Proactive message subscriber (web-only in the desktop runtime —
        // no external channel instances are registered here). Uses a
        // Once-guarded registrar so domain-level startup can't duplicate it.
        crate::openhuman::channels::proactive::register_web_only_proactive_subscriber();

        // Native request handlers — typed in-process request/response.
        // The agent `agent.run_turn` handler is what channel dispatch
        // calls instead of importing `run_tool_call_loop` directly.
        crate::openhuman::agent::bus::register_agent_handlers();

        log::info!(
            "[event_bus] domain subscribers registered (webhook, channel, health, conversation, composio, restart, proactive, agent)"
        );
    });
}

/// Initializes long-lived socket/event-bus infrastructure.
pub async fn bootstrap_skill_runtime() {
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
    // Uses a Once guard so repeated calls to bootstrap_skill_runtime()
    // cannot double-subscribe.
    register_domain_subscribers(workspace_dir.clone(), cfg.clone());

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
        let api_url = crate::api::config::effective_api_url(&config.api_url);
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
    let mut methods = vec![
        HttpMethodSchema {
            method: "core.ping".to_string(),
            namespace: "core".to_string(),
            function: "ping".to_string(),
            description: "Liveness probe for the core JSON-RPC server.".to_string(),
            inputs: vec![],
            outputs: vec![crate::core::FieldSchema {
                name: "ok",
                ty: crate::core::TypeSchema::Bool,
                comment: "Always true when the server is reachable.",
                required: true,
            }],
        },
        HttpMethodSchema {
            method: "core.version".to_string(),
            namespace: "core".to_string(),
            function: "version".to_string(),
            description: "Returns the core binary version.".to_string(),
            inputs: vec![],
            outputs: vec![crate::core::FieldSchema {
                name: "version",
                ty: crate::core::TypeSchema::String,
                comment: "Semantic version string for the running core binary.",
                required: true,
            }],
        },
    ];

    methods.extend(
        all::all_controller_schemas()
            .into_iter()
            .map(|schema| HttpMethodSchema {
                method: all::rpc_method_name(&schema),
                namespace: schema.namespace.to_string(),
                function: schema.function.to_string(),
                description: schema.description.to_string(),
                inputs: schema.inputs,
                outputs: schema.outputs,
            }),
    );

    // Sort methods alphabetically for consistent output.
    methods.sort_by(|a, b| a.method.cmp(&b.method));

    HttpSchemaDump { methods }
}

#[cfg(test)]
#[path = "jsonrpc_tests.rs"]
mod tests;
