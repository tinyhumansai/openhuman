//! Socket.IO event routing and protocol handlers.
//!
//! Thin transport layer: parses incoming Socket.IO events and publishes them
//! to the event bus for domain-specific handling. Webhook routing lives in
//! `webhooks::bus`, channel inbound processing lives in `channels::bus`.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc;

use crate::api::models::socket::ConnectionStatus;
use crate::openhuman::event_bus::{publish_global, DomainEvent};
use crate::openhuman::webhooks::WebhookRequest;

use super::manager::{emit_server_event, emit_state_change, SharedState};

// ---------------------------------------------------------------------------
// Main event dispatcher
// ---------------------------------------------------------------------------

/// Route a Socket.IO event to the appropriate handler based on its name.
pub(super) fn handle_sio_event(
    event_name: &str,
    data: serde_json::Value,
    _emit_tx: &mpsc::UnboundedSender<String>,
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
        // Webhook tunnel — publish to event bus for routing by WebhookRequestSubscriber
        "webhook:request" => {
            log::info!("[socket] Publishing webhook:request to event bus");
            match serde_json::from_value::<WebhookRequest>(data.clone()) {
                Ok(request) => {
                    publish_global(DomainEvent::WebhookIncomingRequest {
                        request,
                        raw_data: data,
                    });
                }
                Err(e) => {
                    log::error!("[socket] Failed to parse webhook:request payload: {e}");
                    // Publish with a minimal request so the subscriber can still
                    // emit an error response. Build a request from what we can parse.
                    let cid = data
                        .get("correlationId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let _tunnel_uuid = data
                        .get("tunnelUuid")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    // Record parse error in router debug log if available
                    if let Some(router) = shared.webhook_router.read().clone() {
                        router.record_parse_error(
                            cid.clone(),
                            data.get("tunnelUuid")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            data.get("method")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            data.get("path")
                                .and_then(|v| v.as_str())
                                .map(|v| v.to_string()),
                            data.clone(),
                            format!("bad request: {e}"),
                        );
                    }

                    // Emit error response directly via socket manager
                    if let Some(mgr) = crate::openhuman::socket::global_socket_manager() {
                        let err_json = json!({ "error": format!("Bad request: {e}") });
                        let body = base64_encode(&err_json.to_string());
                        let response_data = json!({
                            "correlationId": cid,
                            "statusCode": 400,
                            "headers": {},
                            "body": body,
                        });
                        let mgr = mgr.clone();
                        tokio::spawn(async move {
                            if let Err(e) = mgr.emit("webhook:response", response_data).await {
                                log::error!("[socket] Failed to emit webhook error response: {e}");
                            }
                        });
                    }
                }
            }
        }
        // Channel inbound message — publish to event bus for ChannelInboundSubscriber
        _ if event_name.ends_with(":message") => {
            log::info!(
                "[socket] Publishing inbound channel message '{}' to event bus",
                event_name
            );

            let channel = data
                .get("channel")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let message = data
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .trim()
                .to_string();

            if channel.is_empty() {
                log::warn!("[socket] channel:message missing 'channel' field");
                return;
            }
            if message.is_empty() {
                log::debug!("[socket] channel:message empty or missing 'message'");
                return;
            }

            publish_global(DomainEvent::ChannelInboundMessage {
                event_name: event_name.to_string(),
                channel,
                message,
                raw_data: data,
            });
        }
        _ => {
            log::debug!("[socket] Unhandled event '{}' — logging only", event_name);
            emit_server_event(shared, event_name, data);
        }
    }
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Base64-encode a string (for webhook error response bodies).
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
