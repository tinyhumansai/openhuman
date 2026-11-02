//! Server-Sent Event streams: `/events`, `/events/webhooks`, `/events/domain`.

use crate::core_host::core::events::DomainEvent;
use crate::core_host::core::runtime::DispatchCtx;
use axum::extract::Query;
use axum::http::{header, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;
use tokio_stream::StreamExt;

/// Query parameters for the events SSE endpoint.
///
/// `client_id` selects which broadcast events to forward; `token` is the
/// single-shot bind token minted by the `core.events_subscribe_token` RPC.
/// Both are required — browser `EventSource` cannot attach an
/// `Authorization` header, so the bind token is the only credential the
/// endpoint accepts.
#[derive(Debug, serde::Deserialize)]
pub(super) struct EventsQuery {
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
pub(super) async fn events_handler(
    headers: axum::http::HeaderMap,
    ctx: Option<Extension<DispatchCtx>>,
    Query(query): Query<EventsQuery>,
) -> Response {
    // The context the serving middleware attached; a request that carries none
    // has no tenant, which fails closed in SaaS.
    let ctx = ctx.map_or_else(DispatchCtx::unbound, |Extension(ctx)| ctx);
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let bearer_ok = bearer
        .map(crate::core_host::core::auth::verify_bearer_token)
        .unwrap_or(false);

    // SaaS: the stream belongs to the user the gateway scoped this request to,
    // and carries only that user's events. No user scope, or a browser bind
    // token instead of the gateway's bearer, is refused outright.
    // The stream keys on the tenant (profile) of the request's own context.
    let saas_profile = if crate::core_host::core::runtime::is_saas() {
        let profile = ctx.tenant().ok().and_then(|tenant| tenant.profile);
        match profile {
            Some(profile) if bearer_ok => Some(profile),
            _ => {
                log::warn!("[events] reject subscribe: SaaS streams need a gateway user scope");
                return (StatusCode::NOT_FOUND, Json(json!({ "error": "not found" })))
                    .into_response();
            }
        }
    } else {
        None
    };

    if !bearer_ok {
        let supplied_token = query
            .token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let Some(supplied_token) = supplied_token else {
            let client_id_len = query.client_id.len();
            log::warn!(
                "[events] reject subscribe: missing bind token + missing bearer (client_id_len={})",
                client_id_len
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
        if !crate::core_host::core::event_bind_tokens::consume(&query.client_id, supplied_token) {
            let client_id_len = query.client_id.len();
            log::warn!(
                "[events] reject subscribe: bind token invalid or expired (client_id_len={})",
                client_id_len
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
    let rx = crate::core_host::web_chat::subscribe_web_channel_events();
    let stream = tokio_stream::wrappers::BroadcastStream::new(rx).filter_map(
        move |item| -> Option<Result<Event, std::convert::Infallible>> {
            let event = item.ok()?;
            if event.context().client_id != client_id {
                return None;
            }
            if let Some(profile) = saas_profile.as_deref() {
                if !event.belongs_to_profile(profile) {
                    return None;
                }
            }
            let data = serde_json::to_string(&event).ok()?;
            Some(Ok(Event::default().event(event.event_name()).data(data)))
        },
    );

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(10)))
        .into_response()
}

/// Handler for the webhook debug events SSE endpoint.
pub(super) async fn webhook_events_handler() -> Response {
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
pub(super) async fn domain_events_handler(
    headers: axum::http::HeaderMap,
    ctx: Option<Extension<DispatchCtx>>,
) -> Response {
    let ctx = ctx.map_or_else(DispatchCtx::unbound, |Extension(ctx)| ctx);
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let bearer_ok = bearer
        .map(crate::core_host::core::auth::verify_bearer_token)
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
    let es_cfg = ctx
        .load_config()
        .await
        .map(|c| c.dashboard.event_stream)
        .unwrap_or_default();

    let bus = crate::core_host::core::bus::BUS.get();
    if let Some(response) = domain_event_stream_unavailable(es_cfg.enabled, bus.is_some()) {
        return response;
    }
    let bus = bus.expect("enabled event stream has an initialized event bus");

    log::debug!("[events/domain] client connected, streaming domain events");

    // The active workspace, resolved once here so a client that connects
    // mid-life starts out knowing which rows are its own rather than
    // waiting for the next event to tell it (#5966). This is the one place
    // in this handler that can afford the authoritative read — it happens
    // per connection, not per event — and it refills the cache the row
    // stamping below relies on.
    let active_workspace =
        active_workspace_handle(crate::core_host::config::active_workspace_dir().await);

    // Send config as first SSE event so frontend can apply settings.
    let config_event = Event::default().event("config").data(
        serde_json::to_string(&json!({
            "max_entries": es_cfg.max_entries,
            "new_entries": es_cfg.new_entries,
            "active_workspace": active_workspace,
        }))
        .unwrap_or_default(),
    );

    // `BroadcastStream` wraps a raw `broadcast::Receiver`; tinybus hands back a
    // decoding receiver instead, so the stream is built by unfolding it. Lag is
    // already handled inside `recv`, which is why there is no error arm to
    // filter out any more.
    let event_stream = futures::stream::unfold(bus.receiver(), |mut rx| async move {
        rx.recv()
            .await
            .map(|event| (Ok::<_, std::convert::Infallible>(event), rx))
    })
    .filter_map(|item| -> Option<Result<Event, std::convert::Infallible>> {
        let event = item.ok()?;
        domain_event_payload(&event)
            .map(|(domain, data)| Ok(Event::default().event(domain).data(data)))
    });

    let config_stream =
        futures::stream::once(async move { Ok::<_, std::convert::Infallible>(config_event) });
    let stream = config_stream.chain(event_stream);

    Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(5)))
        .into_response()
}

fn domain_event_stream_unavailable(enabled: bool, bus_initialized: bool) -> Option<Response> {
    let (status, error) = if !enabled {
        (StatusCode::NOT_FOUND, "event stream disabled by config")
    } else if !bus_initialized {
        log::warn!("[events/domain] event bus not initialized");
        (StatusCode::SERVICE_UNAVAILABLE, "event bus not initialized")
    } else {
        return None;
    };
    Some((status, Json(json!({ "ok": false, "error": error }))).into_response())
}

fn active_workspace_handle(result: anyhow::Result<std::path::PathBuf>) -> Option<String> {
    result
        .map(|dir| crate::core_host::config::workspace_handle(&dir))
        .map_err(|error| {
            log::warn!(
                "[events/domain] could not resolve the active workspace ({error}); \
                 the client will scope the log once an event says which workspace is active"
            );
        })
        .ok()
}

fn domain_event_payload(event: &DomainEvent) -> Option<(String, String)> {
    let domain = event.domain().to_string();
    let data = json!({
        "domain": domain,
        "event": event.variant_name(),
        "agent": event.agent_hint().unwrap_or(""),
        // Only already-redacted failure details enter the event log.
        "detail": event.log_detail(),
        // Public event rows expose workspace handles, never raw home paths.
        "workspace": event.workspace_dir().map(crate::core_host::config::workspace_handle),
        "active_workspace": crate::core_host::config::active_workspace_dir_cached()
            .map(|dir| crate::core_host::config::workspace_handle(&dir)),
        "timestamp": chrono::Utc::now().format("%H:%M:%S").to_string(),
    });
    serde_json::to_string(&data).ok().map(|data| (domain, data))
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
