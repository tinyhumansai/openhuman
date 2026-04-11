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
/// It parses the request, invokes the requested method, and returns a
/// JSON-RPC 2.0 compliant success or failure response.
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
/// It first checks if the method is registered in the static schema registry.
/// If not, it falls back to the dynamic dispatch system.
///
/// # Arguments
///
/// * `state` - The application state.
/// * `method` - The name of the method to invoke.
/// * `params` - The JSON parameters for the method.
pub async fn invoke_method(state: AppState, method: &str, params: Value) -> Result<Value, String> {
    let result = invoke_method_inner(state, method, params).await;

    // If the RPC call failed due to an expired/invalid session token (401 from
    // the backend), automatically clear the stored session so the frontend
    // detects the logged-out state and redirects to login.
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

fn is_session_expired_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    (lower.contains("401") && lower.contains("unauthorized"))
        || lower.contains("invalid token")
        || msg.contains("SESSION_EXPIRED")
}

async fn invoke_method_inner(
    state: AppState,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    if let Some(schema) = all::schema_for_rpc_method(method) {
        let params_obj = params_to_object(params)?;
        all::validate_params(&schema, &params_obj)?;
        if let Some(result) = all::try_invoke_registered_rpc(method, params_obj).await {
            return result;
        }
        return Err(format!("registered schema has no handler: {method}"));
    }

    crate::core::dispatch::dispatch(state, method, params).await
}

/// Converts JSON parameters into a map, ensuring they are in object format.
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
    let Some(engine) = crate::openhuman::skills::global_engine() else {
        let stream = tokio_stream::once(Ok::<Event, std::convert::Infallible>(
            Event::default()
                .event("webhooks_debug")
                .data("{\"event_type\":\"runtime_unavailable\"}"),
        ));
        return Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
            .into_response();
    };

    let rx = engine.webhook_router().subscribe_debug_events();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(move |item| {
        let event = match item {
            Ok(ev) => ev,
            Err(_) => return None,
        };
        let data = match serde_json::to_string(&event) {
            Ok(data) => data,
            Err(_) => return None,
        };
        Some(Ok::<Event, std::convert::Infallible>(
            Event::default().event("webhooks_debug").data(data),
        ))
    });

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
/// bootstraps the skill runtime, and starts serving requests.
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

    // Optional background bootstrap for local AI services.
    tokio::spawn(async move {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) => {
                if config.local_ai.enabled {
                    let service = crate::openhuman::local_ai::global(&config);
                    service.bootstrap(&config).await;
                }

                if embedded_core {
                    log::debug!("[core] embedded core startup");
                } else {
                    log::debug!("[core] desktop core startup");
                }

                // Start the voice server (records + transcribes) and/or the
                // dictation hotkey listener (broadcasts hotkey events to
                // Socket.IO). Both use rdev::listen() which only supports
                // one instance per process on macOS — so when the voice
                // server is active it owns the single listener and forwards
                // hotkey events to the dictation bus itself.
                crate::openhuman::voice::server::start_if_enabled(&config).await;
                if !config.voice_server.auto_start {
                    crate::openhuman::voice::dictation_listener::start_if_enabled(&config).await;
                }

                // Initialize screen intelligence engine if enabled in config.
                crate::openhuman::screen_intelligence::server::start_if_enabled(&config).await;
                crate::openhuman::autocomplete::start_if_enabled(&config).await;

                // Register autocomplete shutdown hook so the engine (and its
                // Swift overlay helper) are stopped cleanly on process exit.
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

                // Subconscious engine + heartbeat bootstrap is now gated on
                // login so seed_default_tasks() runs against the per-user
                // workspace (`~/.openhuman/users/<id>/workspace/`) instead
                // of the pre-login global workspace. The login handler in
                // `credentials::ops` triggers the same bootstrap after it
                // writes `active_user.toml`.
                //
                // If the user is already logged in from a previous session
                // (active_user.toml exists on disk), kick the bootstrap now
                // so the heartbeat loop starts without waiting for the user
                // to re-authenticate.
                if !config.heartbeat.enabled {
                    log::info!("[subconscious] disabled by config (heartbeat.enabled = false)");
                } else {
                    let already_logged_in = crate::openhuman::config::default_root_openhuman_dir()
                        .ok()
                        .and_then(|root| crate::openhuman::config::read_active_user_id(&root))
                        .is_some();
                    if already_logged_in {
                        match crate::openhuman::subconscious::global::bootstrap_after_login().await
                        {
                            Ok(()) => log::info!(
                                "[subconscious] bootstrapped on startup (existing session)"
                            ),
                            Err(e) => log::warn!("[subconscious] startup bootstrap failed: {e}"),
                        }
                    } else {
                        log::info!("[subconscious] bootstrap deferred — waiting for login");
                    }
                }
            }
            Err(err) => {
                log::warn!("[core] config load failed, skipping local-ai startup: {err}");
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
fn register_domain_subscribers(workspace_dir: std::path::PathBuf) {
    use std::sync::{Arc, Once};

    static REGISTERED: Once = Once::new();
    REGISTERED.call_once(|| {
        // Leak the SubscriptionHandle so the background tasks live for the
        // entire process — SubscriptionHandle::drop aborts the task.
        if let Some(handle) = crate::openhuman::event_bus::subscribe_global(Arc::new(
            crate::openhuman::webhooks::bus::WebhookRequestSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!("[event_bus] failed to register webhook subscriber — bus not initialized");
        }

        if let Some(handle) = crate::openhuman::event_bus::subscribe_global(Arc::new(
            crate::openhuman::channels::bus::ChannelInboundSubscriber::new(),
        )) {
            std::mem::forget(handle);
        } else {
            log::warn!("[event_bus] failed to register channel subscriber — bus not initialized");
        }

        crate::openhuman::health::bus::register_health_subscriber();
        crate::openhuman::skills::bus::register_skill_cleanup_subscriber();
        crate::openhuman::memory::conversations::register_conversation_persistence_subscriber(
            workspace_dir,
        );
        crate::openhuman::composio::register_composio_trigger_subscriber();

        // Restart requests go through a subscriber so every trigger path shares
        // the same respawn logic.
        crate::openhuman::service::bus::register_restart_subscriber();

        log::info!(
            "[event_bus] webhook, channel, health, skill, composio, and restart subscribers registered"
        );
    });
}

/// Initializes the QuickJS skill runtime, socket manager, and registers them
/// globally so RPC handlers (`openhuman.skills_*`, `openhuman.socket_*`) can
/// reach them.
pub async fn bootstrap_skill_runtime() {
    use crate::openhuman::skills::paths::resolve_runtime_paths;
    use crate::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};
    use crate::openhuman::socket::{set_global_socket_manager, SocketManager};
    use std::sync::Arc;

    // Resolve per-user scoped paths via Config so `skills_data` lives under
    // `~/.openhuman/users/{user_id}/skills_data` when an active user is set,
    // matching how config, workspace, and auth are scoped.
    let paths = resolve_runtime_paths().await;
    let skills_data_dir = paths.skills_data_dir.clone();
    let workspace_dir = paths.workspace_dir.clone();

    if let Err(e) = std::fs::create_dir_all(&skills_data_dir) {
        log::error!(
            "[runtime] Failed to create skills data dir {}: {e}",
            skills_data_dir.display()
        );
        return;
    }

    let engine = match RuntimeEngine::new(skills_data_dir) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            log::error!("[runtime] Failed to create RuntimeEngine: {e}");
            return;
        }
    };

    // Point the engine at the (also scoped) workspace directory for
    // user-installed skills.
    let _ = std::fs::create_dir_all(&workspace_dir);
    engine.set_workspace_dir(workspace_dir.clone());

    // --- Event bus bootstrap ---
    // Ensure the global event bus is initialized (no-op if already done by start_channels).
    crate::openhuman::event_bus::init_global(crate::openhuman::event_bus::DEFAULT_CAPACITY);
    // Register domain subscribers for cross-module event handling.
    // Uses a Once guard so repeated calls to bootstrap_skill_runtime()
    // cannot double-subscribe.
    register_domain_subscribers(workspace_dir.clone());

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

    // Register engine globally so RPC handlers can access it.
    set_global_engine(engine.clone());

    // Start the ping scheduler (background health checks).
    engine.ping_scheduler().start();

    // Start the cron scheduler.
    engine.cron_scheduler().start();

    log::info!("[runtime] Skill runtime initialized");

    // Auto-start skills in the background so it doesn't block server startup.
    let engine_for_skills = engine.clone();
    tokio::spawn(async move {
        engine_for_skills.auto_start_skills().await;
    });

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
mod tests {
    use serde_json::json;

    use super::{build_http_schema_dump, default_state, invoke_method};

    #[tokio::test]
    async fn invoke_health_snapshot_via_registry() {
        let result = invoke_method(default_state(), "openhuman.health_snapshot", json!({}))
            .await
            .expect("health snapshot should succeed");
        assert!(result.get("result").is_some());
    }

    #[tokio::test]
    async fn invoke_encrypt_secret_missing_required_param_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.encrypt_secret", json!({}))
            .await
            .expect_err("missing plaintext should fail");
        assert!(err.contains("missing required param 'plaintext'"));
    }

    #[tokio::test]
    async fn invoke_doctor_models_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.doctor_models",
            json!({ "invalid": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'invalid'"));
    }

    #[tokio::test]
    async fn invoke_config_get_runtime_flags_via_registry() {
        let result = invoke_method(
            default_state(),
            "openhuman.config_get_runtime_flags",
            json!({}),
        )
        .await
        .expect("runtime flags should succeed");
        assert!(result.get("result").is_some());
    }

    #[tokio::test]
    async fn invoke_autocomplete_status_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.autocomplete_status",
            json!({ "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'extra'"));
    }

    #[tokio::test]
    async fn invoke_auth_store_session_missing_token_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.auth_store_session", json!({}))
            .await
            .expect_err("missing token should fail");
        assert!(err.contains("missing required param 'token'"));
    }

    #[tokio::test]
    async fn invoke_service_status_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.service_status",
            json!({ "x": 1 }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'x'"));
    }

    #[tokio::test]
    async fn invoke_memory_init_accepts_empty_params() {
        // jwt_token is optional (accepted for backward compat but ignored).
        // The call may still fail for workspace reasons in test, but must NOT
        // fail with a missing-param error for jwt_token.
        let result = invoke_method(default_state(), "openhuman.memory_init", json!({})).await;
        if let Err(ref e) = result {
            assert!(
                !e.contains("missing required param") || !e.contains("jwt_token"),
                "jwt_token should be optional, got: {e}"
            );
        }
    }

    #[tokio::test]
    async fn invoke_memory_list_namespaces_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.memory_list_namespaces",
            json!({ "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("extra"));
    }

    #[tokio::test]
    async fn invoke_memory_query_namespace_missing_namespace_fails() {
        let err = invoke_method(
            default_state(),
            "openhuman.memory_query_namespace",
            json!({ "query": "who owns atlas" }),
        )
        .await
        .expect_err("missing namespace should fail");
        assert!(err.contains("namespace"));
    }

    #[tokio::test]
    async fn invoke_memory_recall_memories_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.memory_recall_memories",
            json!({ "namespace": "team", "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("extra"));
    }

    #[tokio::test]
    async fn invoke_migrate_openclaw_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.migrate_openclaw",
            json!({ "x": 1 }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'x'"));
    }

    #[tokio::test]
    async fn invoke_local_ai_download_asset_missing_required_param_fails_validation() {
        let err = invoke_method(
            default_state(),
            "openhuman.local_ai_download_asset",
            json!({}),
        )
        .await
        .expect_err("missing capability should fail");
        assert!(err.contains("missing required param 'capability'"));
    }

    #[test]
    fn http_schema_dump_includes_openhuman_and_core_methods() {
        let dump = build_http_schema_dump();
        let methods = dump.methods;
        assert!(
            methods
                .iter()
                .any(|m| m.method == "core.version" && m.namespace == "core"),
            "schema dump should include core methods"
        );

        assert!(
            methods
                .iter()
                .any(|m| m.method == "openhuman.health_snapshot"),
            "schema dump should include migrated openhuman methods"
        );

        assert!(
            methods
                .iter()
                .any(|m| m.method == "openhuman.billing_get_current_plan"),
            "schema dump should include billing methods"
        );

        assert!(
            methods
                .iter()
                .any(|m| m.method == "openhuman.team_list_members"),
            "schema dump should include team methods"
        );
    }

    #[tokio::test]
    async fn billing_get_current_plan_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.billing_get_current_plan",
            json!({ "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'extra'"));
    }

    #[tokio::test]
    async fn billing_purchase_plan_missing_plan_fails_validation() {
        let err = invoke_method(
            default_state(),
            "openhuman.billing_purchase_plan",
            json!({}),
        )
        .await
        .expect_err("missing plan should fail");
        assert!(err.contains("missing required param 'plan'"));
    }

    #[tokio::test]
    async fn billing_top_up_missing_amount_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.billing_top_up", json!({}))
            .await
            .expect_err("missing amountUsd should fail");
        assert!(err.contains("missing required param 'amountUsd'"));
    }

    #[tokio::test]
    async fn billing_top_up_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.billing_top_up",
            json!({ "amountUsd": 10.0, "unknownField": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'unknownField'"));
    }

    #[tokio::test]
    async fn billing_create_portal_session_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.billing_create_portal_session",
            json!({ "x": 1 }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'x'"));
    }

    #[tokio::test]
    async fn team_list_members_missing_team_id_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.team_list_members", json!({}))
            .await
            .expect_err("missing teamId should fail");
        assert!(err.contains("missing required param 'teamId'"));
    }

    #[tokio::test]
    async fn team_list_members_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.team_list_members",
            json!({ "teamId": "t1", "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'extra'"));
    }

    #[tokio::test]
    async fn team_create_invite_missing_team_id_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.team_create_invite", json!({}))
            .await
            .expect_err("missing teamId should fail");
        assert!(err.contains("missing required param 'teamId'"));
    }

    #[tokio::test]
    async fn team_remove_member_missing_required_params_fails_validation() {
        let err = invoke_method(
            default_state(),
            "openhuman.team_remove_member",
            json!({ "teamId": "t1" }),
        )
        .await
        .expect_err("missing userId should fail");
        assert!(err.contains("missing required param 'userId'"));
    }

    #[tokio::test]
    async fn team_change_member_role_missing_role_fails_validation() {
        let err = invoke_method(
            default_state(),
            "openhuman.team_change_member_role",
            json!({ "teamId": "t1", "userId": "u1" }),
        )
        .await
        .expect_err("missing role should fail");
        assert!(err.contains("missing required param 'role'"));
    }

    #[tokio::test]
    async fn billing_create_coinbase_charge_missing_plan_fails_validation() {
        let err = invoke_method(
            default_state(),
            "openhuman.billing_create_coinbase_charge",
            json!({}),
        )
        .await
        .expect_err("missing plan should fail");
        assert!(err.contains("missing required param 'plan'"));
    }

    #[tokio::test]
    async fn billing_create_coinbase_charge_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.billing_create_coinbase_charge",
            json!({ "plan": "pro", "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'extra'"));
    }

    #[tokio::test]
    async fn team_list_invites_missing_team_id_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.team_list_invites", json!({}))
            .await
            .expect_err("missing teamId should fail");
        assert!(err.contains("missing required param 'teamId'"));
    }

    #[tokio::test]
    async fn team_list_invites_rejects_unknown_param() {
        let err = invoke_method(
            default_state(),
            "openhuman.team_list_invites",
            json!({ "teamId": "t1", "extra": true }),
        )
        .await
        .expect_err("unknown param should fail");
        assert!(err.contains("unknown param 'extra'"));
    }

    #[tokio::test]
    async fn team_revoke_invite_missing_team_id_fails_validation() {
        let err = invoke_method(default_state(), "openhuman.team_revoke_invite", json!({}))
            .await
            .expect_err("missing teamId should fail");
        assert!(err.contains("missing required param 'teamId'"));
    }

    #[tokio::test]
    async fn team_revoke_invite_missing_invite_id_fails_validation() {
        let err = invoke_method(
            default_state(),
            "openhuman.team_revoke_invite",
            json!({ "teamId": "t1" }),
        )
        .await
        .expect_err("missing inviteId should fail");
        assert!(err.contains("missing required param 'inviteId'"));
    }

    #[tokio::test]
    async fn schema_dump_includes_new_billing_and_team_methods() {
        let dump = build_http_schema_dump();
        let methods: Vec<&str> = dump.methods.iter().map(|m| m.method.as_str()).collect();
        for expected in &[
            "openhuman.billing_get_current_plan",
            "openhuman.billing_purchase_plan",
            "openhuman.billing_create_portal_session",
            "openhuman.billing_top_up",
            "openhuman.billing_create_coinbase_charge",
            "openhuman.team_list_members",
            "openhuman.team_create_invite",
            "openhuman.team_list_invites",
            "openhuman.team_revoke_invite",
            "openhuman.team_remove_member",
            "openhuman.team_change_member_role",
        ] {
            assert!(
                methods.contains(expected),
                "schema dump missing expected method: {expected}"
            );
        }
    }
}
