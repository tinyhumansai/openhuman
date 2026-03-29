use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::core_server::types::AppState;

/// Full HTTP app (/, /health, /rpc) for the core process and for integration tests.
pub fn build_core_http_router() -> Router {
    Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/rpc", post(crate::core::jsonrpc::rpc_handler))
        .fallback(not_found_handler)
        .with_state(AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        })
}

async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "ok": true })))
}

async fn root_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(json!({
            "name": "openhuman",
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

    let app = build_core_http_router();

    log::info!("[core] listening on http://{bind_addr}");
    log::info!("[rpc:http] JSON-RPC server running — POST http://{bind_addr}/rpc (JSON-RPC 2.0)");
    log::info!(
        "[core] JSON-RPC log markers: [rpc:http] (HTTP), [rpc:dispatch] (router), [rpc:call] (CLI). RUST_LOG=debug for redacted params + subsystem; trace for response bodies."
    );

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
