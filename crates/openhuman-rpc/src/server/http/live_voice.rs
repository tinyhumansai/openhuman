//! `GET /ws/live-voice`: the authenticated live voice agent WebSocket.
//!
//! Upgrade guard uses the origin allowlist plus the core bearer from the
//! `Authorization` header or `?token=`; the session itself is
//! `openhuman::voice::live::ws::handle_live_voice_ws`.

use std::sync::Arc;

use axum::extract::{Extension, WebSocketUpgrade};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;

use crate::core_host::core::runtime::DispatchCtx;

/// WebSocket upgrade handler for live voice sessions.
pub(super) async fn live_voice_ws_handler(
    headers: HeaderMap,
    ctx: Option<Extension<DispatchCtx>>,
    ws: WebSocketUpgrade,
) -> Response {
    log::info!("[ws] live voice WebSocket upgrade requested");
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::trim);
    if !crate::server::socketio::origin_is_allowed(origin) {
        log::warn!("[ws] live voice upgrade rejected: disallowed origin {origin:?}");
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "ok": false,
                "error": "forbidden",
                "message": "Origin not allowed for the live voice WebSocket.",
            })),
        )
            .into_response();
    }
    // The context the serving middleware attached: the upgraded socket runs on
    // its own task, so it cannot be read back from the request task. A request
    // that carries none has no tenant, which fails closed in SaaS.
    let ctx = ctx.map_or_else(DispatchCtx::unbound, |Extension(ctx)| ctx);
    // Agent audio arrives in ~40 ms chunks; keep frames small and unbounded
    // in count rather than buffering.
    ws.on_upgrade(|socket| async move {
        let config = match ctx.load_config().await {
            Ok(c) => Arc::new(c),
            Err(e) => {
                log::error!("[ws] failed to load config for live voice: {e}");
                return;
            }
        };
        crate::core_host::voice::live::ws::handle_live_voice_ws(socket, ctx, config).await;
    })
}
