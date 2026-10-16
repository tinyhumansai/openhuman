//! The core's axum HTTP transport, compiled only with the `http-server` feature.
//!
//! [`build_core_http_router`] assembles every route; each route family has its
//! own module. Binding the listener and running the server belongs to
//! [`serve`](super::serve).

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::json;

use crate::core_host::core::types::AppState;

pub(crate) mod cors;
mod dictation;
mod events;
mod health;
mod live_voice;
mod oauth_mcp;
mod pages;
mod rpc_handler;

pub use rpc_handler::rpc_handler;

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
    crate::http_host::ensure_registered();
    let router = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health::health_handler))
        .route("/schema", get(health::schema_handler))
        .route("/events", get(events::events_handler))
        .route("/events/webhooks", get(events::webhook_events_handler))
        .route("/events/domain", get(events::domain_events_handler))
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
            post(rpc_handler::rpc_handler).route_layer(DefaultBodyLimit::max(MAX_RPC_BODY_BYTES)),
        )
        .route("/ws/dictation", get(dictation::dictation_ws_handler))
        .route("/ws/live-voice", get(live_voice::live_voice_ws_handler))
        .route(
            "/oauth/mcp/callback",
            get(oauth_mcp::oauth_mcp_callback_handler),
        )
        // Dev-only: hand this core (URL + bearer) to a loopback Vite renderer.
        .route(
            "/dev/connect",
            get(crate::server::dev_connect::dev_connect_handler),
        )
        // OpenAI-compatible inference endpoint (/v1/chat/completions, /v1/models)
        .nest("/v1", crate::core_host::inference::http::router())
        // Apply `AppState` here so the outer router becomes `Router<()>` and
        // matches any state-less sub-router merged into it.
        .with_state(AppState {
            core_version: env!("CARGO_PKG_VERSION").to_string(),
        });

    let router = router
        .fallback(not_found_handler)
        .layer(middleware::from_fn(http_request_log_middleware))
        .layer(middleware::from_fn(
            crate::server::auth::rpc_auth_middleware,
        ));
    // Socket.IO is off (`--jsonrpc-only`): answer its path before the bearer
    // middleware can mistake the handshake for an unauthenticated request.
    // Sits inside CORS so browser clients can read the body.
    let router = if socketio_enabled {
        router
    } else {
        router.layer(middleware::from_fn(socketio_disabled_middleware))
    };
    let router = router.layer(middleware::from_fn(cors::cors_middleware));

    if socketio_enabled {
        let (socket_layer, io) = crate::server::socketio::attach_socketio();
        crate::server::socketio::spawn_web_channel_bridge(io);
        return router.layer(socket_layer);
    }

    router
}

/// Stable machine-readable `error` code of the "Socket.IO is off" response.
/// Clients (the app's connection test) match on it.
pub const SOCKETIO_DISABLED_ERROR: &str = "socketio_disabled";

/// Replies `503` with a JSON body naming the cause to any `/socket.io` request.
///
/// Only installed when Socket.IO is disabled. Without it the request falls
/// through to the bearer middleware, which logs and returns a misleading 401
/// (the Socket.IO handshake carries its token in the `auth` payload, never in
/// an `Authorization` header) (#5656).
async fn socketio_disabled_middleware(req: Request, next: Next) -> Response {
    let path = req.uri().path();
    if path == "/socket.io" || path.starts_with("/socket.io/") {
        log::info!(
            "[http] {} {} -> 503 socketio_disabled (core started with --jsonrpc-only; realtime is off)",
            req.method(),
            path
        );
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "ok": false,
                "error": SOCKETIO_DISABLED_ERROR,
                "message": "Socket.IO (realtime) is disabled on this core. It was started with --jsonrpc-only; restart it without that flag to enable realtime chat and events."
            })),
        )
            .into_response();
    }
    next.run(req).await
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

/// Handler for the root endpoint, returning server information and available endpoints.
async fn root_handler() -> impl IntoResponse {
    let api_server = match crate::core_host::config::Config::load_or_init().await {
        Ok(cfg) => cfg.resolved_endpoints.control_plane_url.clone(),
        Err(_) => None,
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

#[cfg(test)]
#[path = "inference_route_tests.rs"]
mod inference_route_tests;

#[cfg(test)]
#[path = "socketio_disabled_tests.rs"]
mod socketio_disabled_tests;
