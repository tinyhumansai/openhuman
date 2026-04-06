//! Socket.IO event routing and protocol handlers.
//!
//! Dispatches incoming Socket.IO events to the appropriate handler:
//! webhook tunnel requests, channel inbound messages, or generic event logging.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc;

use crate::api::models::socket::ConnectionStatus;
use crate::openhuman::webhooks::WebhookRequest;

use super::manager::{emit_server_event, emit_state_change, SharedState};

// ---------------------------------------------------------------------------
// Main event dispatcher
// ---------------------------------------------------------------------------

/// Route a Socket.IO event to the appropriate handler based on its name.
pub(super) fn handle_sio_event(
    event_name: &str,
    data: serde_json::Value,
    emit_tx: &mpsc::UnboundedSender<String>,
    shared: &Arc<SharedState>,
) {
    // Log every incoming event for observability.
    log::info!(
        "[socket] event received: name={} data_bytes={}",
        event_name,
        data.to_string().len()
    );
    log::debug!(
        "[socket] event payload: name={} data={}",
        event_name,
        &data.to_string()[..data.to_string().len().min(500)]
    );

    match event_name {
        "ready" => {
            log::info!("[socket] Server ready — auth successful");
            *shared.status.write() = ConnectionStatus::Connected;
            emit_state_change(shared);
        }
        "error" => {
            log::error!("[socket] Server error event: {}", data);
            *shared.status.write() = ConnectionStatus::Error;
            emit_state_change(shared);
        }
        // Webhook tunnel — route to owning skill and relay response
        "webhook:request" => {
            log::info!("[socket] Routing webhook:request to handler");
            let shared = Arc::clone(shared);
            let tx = emit_tx.clone();
            tokio::spawn(async move {
                handle_webhook_request(&shared, data, &tx).await;
            });
        }
        // Any event ending with ":message" is treated as an inbound channel
        // message that triggers the agent loop. This covers channel:message,
        // telegram:message, discord:message, slack:message, and any future
        // channel integration without requiring code changes.
        _ if event_name.ends_with(":message") => {
            log::info!(
                "[socket] Inbound channel message via '{}' — triggering agent loop",
                event_name
            );
            tokio::spawn(async move {
                handle_channel_inbound_message(data).await;
            });
        }
        _ => {
            log::debug!("[socket] Unhandled event '{}' — logging only", event_name);
            emit_server_event(shared, event_name, data);
        }
    }
}

// ---------------------------------------------------------------------------
// Webhook tunnel handler
// ---------------------------------------------------------------------------

/// Handle an incoming `webhook:request` event from the backend.
///
/// Routes the request to the owning skill via the WebhookRouter, waits for the
/// skill's response, and emits `webhook:response` back through the socket.
async fn handle_webhook_request(
    shared: &SharedState,
    data: serde_json::Value,
    emit_tx: &mpsc::UnboundedSender<String>,
) {
    let request: WebhookRequest = match serde_json::from_value(data.clone()) {
        Ok(r) => r,
        Err(e) => {
            log::error!("[socket] Failed to parse webhook:request payload: {e}");
            let cid = data
                .get("correlationId")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            if let Some(router) = shared.webhook_router.read().clone() {
                router.record_parse_error(
                    cid.clone(),
                    data.get("tunnelUuid")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    data.get("method")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    data.get("path")
                        .and_then(|value| value.as_str())
                        .map(|value| value.to_string()),
                    data.clone(),
                    format!("bad request: {e}"),
                );
            }
            emit_via_channel(
                emit_tx,
                "webhook:response",
                json!({
                    "correlationId": cid,
                    "statusCode": 400,
                    "headers": {},
                    "body": base64_encode(&format!(
                        "{{\"error\":\"Bad request: {}\"}}",
                        e.to_string().replace('"', "\\\"")
                    )),
                }),
            );
            return;
        }
    };

    let correlation_id = request.correlation_id.clone();
    let tunnel_uuid = request.tunnel_uuid.clone();
    let tunnel_name = request.tunnel_name.clone();
    let method = request.method.clone();
    let path = request.path.clone();

    log::info!(
        "[socket] webhook:request {} {} (tunnel={}, correlationId={})",
        method,
        path,
        tunnel_uuid,
        correlation_id,
    );

    let router = shared.webhook_router.read().clone();
    let registration = router.as_ref().and_then(|r| r.registration(&tunnel_uuid));
    let skill_id = registration.as_ref().and_then(|registration| {
        (registration.target_kind == "skill").then(|| registration.skill_id.clone())
    });
    if let Some(router) = router.as_ref() {
        router.record_request(&request, skill_id.clone());
    }

    let (response, resolved_skill_id, response_error) = match registration.as_ref() {
        Some(registration) if registration.target_kind == "echo" => (
            crate::openhuman::webhooks::ops::build_echo_response(&request),
            Some("echo".to_string()),
            None,
        ),
        Some(registration) if registration.target_kind == "channel" => (
            crate::openhuman::webhooks::WebhookResponseData {
                correlation_id: correlation_id.clone(),
                status_code: 501,
                headers: std::collections::HashMap::new(),
                body: base64_encode(&format!(
                    "{{\"error\":\"channel webhook target '{}' is not implemented in this runtime yet\"}}",
                    registration.skill_id.replace('"', "\\\"")
                )),
            },
            Some(registration.skill_id.clone()),
            Some("channel webhook target not implemented".to_string()),
        ),
        Some(registration) if registration.target_kind == "skill" => {
            let sid = registration.skill_id.clone();
            log::debug!("[socket] webhook:request routed to skill '{}'", sid);

            let registry = crate::openhuman::skills::global_engine()
                .map(|e| e.registry());
            match registry {
                Some(registry) => {
                    let result = registry
                        .send_webhook_request(
                            &sid,
                            correlation_id.clone(),
                            request.method.clone(),
                            request.path.clone(),
                            request.headers.clone(),
                            request.query.clone(),
                            request.body.clone(),
                            request.tunnel_id.clone(),
                            request.tunnel_name.clone(),
                        )
                        .await;

                    match result {
                        Ok(resp) => (resp, Some(sid), None),
                        Err(e) => {
                            log::warn!("[socket] Skill webhook handler error: {}", e);
                            (
                                crate::openhuman::webhooks::WebhookResponseData {
                                    correlation_id: correlation_id.clone(),
                                    status_code: 500,
                                    headers: std::collections::HashMap::new(),
                                    body: base64_encode(&format!(
                                        "{{\"error\":\"Skill handler error: {}\"}}",
                                        e.replace('"', "\\\"")
                                    )),
                                },
                                Some(sid),
                                Some(e),
                            )
                        }
                    }
                }
                None => {
                    log::warn!("[socket] No skill registry available for webhook");
                    (
                        crate::openhuman::webhooks::WebhookResponseData {
                            correlation_id: correlation_id.clone(),
                            status_code: 503,
                            headers: std::collections::HashMap::new(),
                            body: base64_encode("{\"error\":\"Runtime not ready\"}"),
                        },
                        None,
                        Some("runtime not ready".to_string()),
                    )
                }
            }
        }
        Some(registration) => (
            crate::openhuman::webhooks::WebhookResponseData {
                correlation_id: correlation_id.clone(),
                status_code: 400,
                headers: std::collections::HashMap::new(),
                body: base64_encode(&format!(
                    "{{\"error\":\"unknown webhook target kind '{}'\"}}",
                    registration.target_kind.replace('"', "\\\"")
                )),
            },
            Some(registration.skill_id.clone()),
            Some("unknown webhook target kind".to_string()),
        ),
        None => {
            log::debug!(
                "[socket] No skill registered for tunnel {}",
                tunnel_uuid,
            );
            (
                crate::openhuman::webhooks::WebhookResponseData {
                    correlation_id: correlation_id.clone(),
                    status_code: 404,
                    headers: std::collections::HashMap::new(),
                    body: base64_encode("{\"error\":\"No handler registered for this tunnel\"}"),
                },
                None,
                Some("no handler registered for this tunnel".to_string()),
            )
        }
    };

    if let Some(router) = router.as_ref() {
        router.record_response(
            &request,
            &response,
            resolved_skill_id.clone(),
            response_error.clone(),
        );
    }

    emit_via_channel(
        emit_tx,
        "webhook:response",
        json!({
            "correlationId": response.correlation_id,
            "statusCode": response.status_code,
            "headers": response.headers,
            "body": response.body,
        }),
    );

    log::info!(
        "[socket] webhook activity: {} {} → status={}, skill={:?}, tunnel={}",
        method,
        path,
        response.status_code,
        resolved_skill_id,
        tunnel_name,
    );
}

// ---------------------------------------------------------------------------
// Channel inbound message → agent loop → reply
// ---------------------------------------------------------------------------

/// Handle an inbound message from a channel (Telegram, Discord, etc.).
///
/// Runs the agent inference loop via `web::start_chat` and sends the response
/// back to the originating channel via the REST API.
async fn handle_channel_inbound_message(data: serde_json::Value) {
    let channel = match data.get("channel").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            log::warn!("[channel-inbound] channel:message missing 'channel' field");
            return;
        }
    };
    let message = match data.get("message").and_then(|v| v.as_str()) {
        Some(m) if !m.trim().is_empty() => m.trim().to_string(),
        _ => {
            log::debug!("[channel-inbound] channel:message empty or missing 'message'");
            return;
        }
    };

    log::info!(
        "[channel-inbound] received message from channel='{}' len={}",
        channel,
        message.len()
    );

    let thread_id = format!("channel:{}", channel);
    let client_id = "inbound".to_string();

    let mut event_rx = crate::openhuman::channels::providers::web::subscribe_web_channel_events();

    let request_id = match crate::openhuman::channels::providers::web::start_chat(
        &client_id, &thread_id, &message, None, None,
    )
    .await
    {
        Ok(rid) => {
            log::debug!(
                "[channel-inbound] agent started request_id={} thread={}",
                rid,
                thread_id
            );
            rid
        }
        Err(err) => {
            log::error!("[channel-inbound] start_chat failed: {}", err);
            send_channel_reply(
                &channel,
                &format!("Sorry, I couldn't process your message: {err}"),
            )
            .await;
            return;
        }
    };

    let timeout = tokio::time::Duration::from_secs(180);
    let deadline = tokio::time::Instant::now() + timeout;

    loop {
        tokio::select! {
            event = event_rx.recv() => {
                match event {
                    Ok(ev) if ev.request_id == request_id => {
                        if ev.event == "chat_done" || ev.event == "chat:done" {
                            let reply = ev.full_response.unwrap_or_default();
                            if reply.trim().is_empty() {
                                log::warn!("[channel-inbound] agent returned empty response");
                                return;
                            }
                            log::info!(
                                "[channel-inbound] agent done, replying to channel='{}' len={}",
                                channel,
                                reply.len()
                            );
                            send_channel_reply(&channel, &reply).await;
                            return;
                        }
                        if ev.event == "chat_error" || ev.event == "chat:error" {
                            let err_msg = ev.message.unwrap_or_else(|| "unknown error".to_string());
                            log::error!("[channel-inbound] agent error: {}", err_msg);
                            send_channel_reply(
                                &channel,
                                &format!("Sorry, I encountered an error: {err_msg}"),
                            )
                            .await;
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        log::warn!("[channel-inbound] event bus lagged, skipped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        log::error!("[channel-inbound] event bus closed unexpectedly");
                        return;
                    }
                }
            }
            _ = tokio::time::sleep_until(deadline) => {
                log::error!("[channel-inbound] agent timed out after {}s", timeout.as_secs());
                send_channel_reply(&channel, "Sorry, the request timed out.").await;
                return;
            }
        }
    }
}

/// Send a text reply back to a channel via the backend REST API.
async fn send_channel_reply(channel: &str, text: &str) {
    let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            log::error!("[channel-inbound] failed to load config: {}", e);
            return;
        }
    };

    let api_url = crate::api::config::effective_api_url(&config.api_url);
    let jwt = match crate::api::jwt::get_session_token(&config) {
        Ok(Some(t)) => t,
        Ok(None) => {
            log::error!("[channel-inbound] no session JWT — cannot reply");
            return;
        }
        Err(e) => {
            log::error!("[channel-inbound] failed to get session token: {}", e);
            return;
        }
    };

    let client = match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => c,
        Err(e) => {
            log::error!("[channel-inbound] failed to create API client: {}", e);
            return;
        }
    };

    let body = json!({ "text": text });
    match client.send_channel_message(channel, &jwt, body).await {
        Ok(resp) => {
            log::info!(
                "[channel-inbound] reply sent to channel='{}' response={:?}",
                channel,
                resp
            );
        }
        Err(e) => {
            log::error!(
                "[channel-inbound] failed to send reply to channel='{}': {}",
                channel,
                e
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Base64-encode a string (for webhook response bodies).
fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

/// Send a Socket.IO event through the emit channel.
///
/// Format: `42["eventName", data]`
pub(super) fn emit_via_channel(
    tx: &mpsc::UnboundedSender<String>,
    event: &str,
    data: serde_json::Value,
) {
    let payload = serde_json::to_string(&json!([event, data])).unwrap_or_default();
    let msg = format!("42{}", payload);
    if let Err(e) = tx.send(msg) {
        log::error!("[socket] emit_via_channel failed: {e}");
    }
}

// ---------------------------------------------------------------------------
// SIO event parsing
// ---------------------------------------------------------------------------

/// Parse a Socket.IO EVENT payload into an event name and JSON data.
///
/// Format: `["eventName", data]` or `<ackId>["eventName", data]`.
pub(super) fn parse_sio_event(text: &str) -> Option<(String, serde_json::Value)> {
    let json_start = text.find('[')?;
    let json_str = &text[json_start..];
    let arr: Vec<serde_json::Value> = serde_json::from_str(json_str).ok()?;
    let event_name = arr.first()?.as_str()?.to_string();
    let data = arr.get(1).cloned().unwrap_or(serde_json::Value::Null);
    Some((event_name, data))
}
