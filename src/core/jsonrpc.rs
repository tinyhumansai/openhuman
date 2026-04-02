use axum::extract::{Query, State};
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

pub async fn rpc_handler(State(state): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    let id = req.id.clone();
    match invoke_method(state, req.method.as_str(), req.params).await {
        Ok(value) => (
            StatusCode::OK,
            Json(RpcSuccess {
                jsonrpc: "2.0",
                id,
                result: value,
            }),
        )
            .into_response(),
        Err(message) => (
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
            .into_response(),
    }
}

pub async fn invoke_method(state: AppState, method: &str, params: Value) -> Result<Value, String> {
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

pub fn parse_json_params(raw: &str) -> Result<Value, String> {
    serde_json::from_str(raw).map_err(|e| format!("invalid JSON params: {e}"))
}

pub fn default_state() -> AppState {
    AppState {
        core_version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

// --- HTTP server (Axum) ----------------------------------------------------

#[derive(Debug, serde::Deserialize)]
struct TelegramAuthQuery {
    token: Option<String>,
}

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

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

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

pub fn build_core_http_router(socketio_enabled: bool) -> Router {
    let router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/schema", get(schema_handler))
        .route("/events", get(events_handler))
        .route("/rpc", post(rpc_handler))
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

async fn http_request_log_middleware(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let query_len = req.uri().query().map(str::len).unwrap_or(0);
    let started = std::time::Instant::now();

    let response = next.run(req).await;

    let status = response.status().as_u16();
    let ms = started.elapsed().as_millis();
    log::info!(
        "[http] {} {}{} -> {} ({}ms)",
        method,
        path,
        if query_len > 0 { "?…" } else { "" },
        status,
        ms
    );

    response
}

async fn cors_middleware(req: Request, next: Next) -> Response {
    if req.method() == Method::OPTIONS {
        return with_cors_headers(StatusCode::NO_CONTENT.into_response());
    }

    let response = next.run(req).await;
    with_cors_headers(response)
}

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

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

async fn schema_handler(State(_state): State<AppState>) -> impl IntoResponse {
    (StatusCode::OK, Json(build_http_schema_dump())).into_response()
}

#[derive(Debug, serde::Deserialize)]
struct EventsQuery {
    client_id: String,
}

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

fn core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

fn core_host() -> String {
    std::env::var("OPENHUMAN_CORE_HOST")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

pub async fn run_server(
    host: Option<&str>,
    port: Option<u16>,
    socketio_enabled: bool,
) -> anyhow::Result<()> {
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

    tokio::spawn(async {
        match crate::openhuman::config::Config::load_or_init().await {
            Ok(config) if config.local_ai.enabled => {
                let service = crate::openhuman::local_ai::global(&config);
                service.bootstrap(&config).await;
            }
            Ok(_) => {}
            Err(err) => {
                log::warn!("[core] local-ai bootstrap skipped: {err}");
            }
        }
    });

    axum::serve(listener, app).await?;
    Ok(())
}

/// Initialize the QuickJS skill runtime and register it globally so RPC
/// handlers (`openhuman.skills_*`) can reach it.
pub async fn bootstrap_skill_runtime() {
    use crate::openhuman::skills::qjs_engine::{set_global_engine, RuntimeEngine};
    use std::sync::Arc;

    // Resolve the base directory (~/.openhuman or $OPENHUMAN_WORKSPACE).
    let base_dir = std::env::var("OPENHUMAN_WORKSPACE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("."))
                .join(".openhuman")
        });

    let skills_data_dir = base_dir.join("skills_data");
    if let Err(e) = std::fs::create_dir_all(&skills_data_dir) {
        log::error!("[runtime] Failed to create skills data dir: {e}");
        return;
    }

    let engine = match RuntimeEngine::new(skills_data_dir) {
        Ok(e) => Arc::new(e),
        Err(e) => {
            log::error!("[runtime] Failed to create RuntimeEngine: {e}");
            return;
        }
    };

    // Point the engine at the workspace directory for user-installed skills.
    let workspace_dir = base_dir.join("workspace");
    let _ = std::fs::create_dir_all(&workspace_dir);
    engine.set_workspace_dir(workspace_dir);

    // Register globally so RPC handlers can access it.
    set_global_engine(engine.clone());

    // Start the ping scheduler (background health checks).
    engine.ping_scheduler().start();

    // Start the cron scheduler.
    engine.cron_scheduler().start();

    log::info!("[runtime] Skill runtime initialized");

    // Auto-start skills in the background so it doesn't block server startup.
    tokio::spawn(async move {
        engine.auto_start_skills().await;
    });
}

#[derive(Serialize)]
struct HttpSchemaDump {
    methods: Vec<HttpMethodSchema>,
}

#[derive(Serialize)]
struct HttpMethodSchema {
    method: String,
    namespace: String,
    function: String,
    description: String,
    inputs: Vec<crate::core::FieldSchema>,
    outputs: Vec<crate::core::FieldSchema>,
}

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
    async fn invoke_memory_init_missing_required_param_fails() {
        let err = invoke_method(default_state(), "openhuman.memory_init", json!({}))
            .await
            .expect_err("missing jwt_token should fail");
        assert!(err.contains("jwt_token"));
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
