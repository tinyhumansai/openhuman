//! `GET /ws/live-voice`: the authenticated live voice agent WebSocket.
//!
//! Same upgrade guard as `/ws/dictation` (origin allowlist plus the core bearer
//! from the `Authorization` header or `?token=`); the session itself is
//! `openhuman::voice::live::ws::handle_live_voice_ws`.

use std::sync::Arc;

use axum::extract::{Extension, Query, WebSocketUpgrade};
use axum::response::{IntoResponse, Response};

use crate::core_host::core::runtime::DispatchCtx;

use super::dictation::{authorize_dictation_request, DictationQuery};

/// WebSocket upgrade handler for live voice sessions.
pub(super) async fn live_voice_ws_handler(
    headers: axum::http::HeaderMap,
    ctx: Option<Extension<DispatchCtx>>,
    Query(query): Query<DictationQuery>,
    ws: WebSocketUpgrade,
) -> Response {
    log::info!("[ws] live voice WebSocket upgrade requested");
    if let Err(error) = authorize_dictation_request(&headers, &query) {
        return error.into_response();
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
