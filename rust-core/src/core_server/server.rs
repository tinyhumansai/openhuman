use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::core_server::dispatch;
use crate::core_server::types::{AppState, RpcFailure, RpcRequest, RpcSuccess, RpcError};

pub async fn rpc_handler(State(state): State<AppState>, Json(req): Json<RpcRequest>) -> Response {
    let id = req.id.clone();

    let result = dispatch::dispatch(state, req.method.as_str(), req.params).await;

    match result {
        Ok(value) => to_rpc_success(id, value),
        Err(message) => rpc_error_response(id, -32000, message),
    }
}

fn rpc_error_response(id: serde_json::Value, code: i64, message: String) -> Response {
    (
        StatusCode::OK,
        Json(RpcFailure {
            jsonrpc: "2.0",
            id,
            error: RpcError {
                code,
                message,
                data: None,
            },
        }),
    )
        .into_response()
}

fn to_rpc_success(id: serde_json::Value, value: serde_json::Value) -> Response {
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

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

async fn root_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "name": "openhuman-core",
            "ok": true,
            "endpoints": {
                "health": "/health",
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
            "message": "Route not found. Try /, /health, or /rpc."
        })),
    )
}

fn core_port() -> u16 {
    std::env::var("OPENHUMAN_CORE_PORT")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(7788)
}

pub async fn run_server(port: Option<u16>) -> anyhow::Result<()> {
    let port = port.unwrap_or_else(core_port);
    let bind_addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/rpc", post(rpc_handler))
        .fallback(not_found_handler)
        .with_state(AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        });

    log::info!("[core] listening on http://{bind_addr}");

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
