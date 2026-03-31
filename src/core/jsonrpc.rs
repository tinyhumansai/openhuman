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

pub fn build_core_http_router(socketio_enabled: bool) -> Router {
    let router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/schema", get(schema_handler))
        .route("/events", get(events_handler))
        .route("/rpc", post(rpc_handler))
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

pub async fn run_server(port: Option<u16>, socketio_enabled: bool) -> anyhow::Result<()> {
    let _ = all::all_registered_controllers();
    let port = port.unwrap_or_else(core_port);
    let bind_addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

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
    }
}
