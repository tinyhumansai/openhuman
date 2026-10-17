//! Socket.IO event routing and protocol handlers.
//!
//! Thin transport layer: parses incoming Socket.IO events and publishes them
//! to the event bus for domain-specific handling. Webhook routing lives in
//! `webhooks::bus`, channel inbound processing lives in `channels::bus`.

use std::sync::Arc;

use serde_json::json;
use tokio::sync::mpsc;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::platform::socket::models::ConnectionStatus;
use crate::skills::webhooks::WebhookRequest;

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
    // Payload content is intentionally omitted from logs — webhook bodies,
    // channel messages, and Composio trigger payloads can carry PII, secrets,
    // or auth tokens. The byte-length alone is sufficient for diagnosing
    // truncation and throughput issues without exposing raw content.
    let payload = data.to_string();
    log::info!(
        "[socket] event received: name={} data_bytes={}",
        event_name,
        payload.len()
    );
    // CodeRabbit #3250222027: even at debug level, raw bodies can leak
    // PII / secrets / tokens. Log structural metadata (top-level shape +
    // byte length) but never the raw text.
    let payload_shape = match &data {
        serde_json::Value::Object(map) => format!("object_keys={}", map.len()),
        serde_json::Value::Array(arr) => format!("array_len={}", arr.len()),
        serde_json::Value::String(_) => "string".to_string(),
        serde_json::Value::Number(_) => "number".to_string(),
        serde_json::Value::Bool(_) => "bool".to_string(),
        serde_json::Value::Null => "null".to_string(),
    };
    log::debug!(
        "[socket] event payload: name={} data_bytes={} shape={} preview_omitted=true",
        event_name,
        payload.len(),
        payload_shape
    );
    log::debug!("[socket] event dispatch: name={}", event_name);

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
                    BUS.publish(DomainEvent::WebhookIncomingRequest {
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
                    if let Some(mgr) = crate::platform::socket::global_socket_manager() {
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
        // Composio trigger webhook — backend emits this after HMAC-verifying
        // an incoming Composio webhook. Deserialize into the canonical
        // `ComposioTriggerEvent` DTO so shape mismatches fail fast with a
        // clear log line instead of being silently coerced to empty strings.
        "composio:trigger" => {
            log::info!("[socket] Publishing composio:trigger to event bus");
            match serde_json::from_value::<crate::integrations::composio::ComposioTriggerEvent>(
                data.clone(),
            ) {
                Ok(event) => {
                    if event.toolkit.is_empty() || event.trigger.is_empty() {
                        log::warn!(
                            "[socket] composio:trigger missing toolkit/trigger; dropping event"
                        );
                    } else {
                        log::info!(
                            "[socket] Publishing composio:trigger to event bus: toolkit={}, trigger={}, metadata_id={}, metadata_uuid={}",
                            event.toolkit,
                            event.trigger,
                            event.metadata.id,
                            event.metadata.uuid
                        );
                        BUS.publish(DomainEvent::ComposioTriggerReceived {
                            toolkit: event.toolkit,
                            trigger: event.trigger,
                            metadata_id: event.metadata.id,
                            metadata_uuid: event.metadata.uuid,
                            payload: event.payload,
                        });
                    }
                }
                Err(e) => {
                    log::warn!(
                        "[socket] failed to parse composio:trigger payload: {e}; dropping event"
                    );
                }
            }
        }
        // Device tunnel — peer-status update.
        "tunnel:peer-status" => {
            log::info!("[socket] tunnel:peer-status received");
            match serde_json::from_value::<crate::security::devices::tunnel_client::TunnelPeerStatus>(
                data.clone(),
            ) {
                Ok(status) => {
                    if status.online {
                        BUS.publish(DomainEvent::DevicePeerOnline {
                            channel_id: status.channel_id,
                        });
                    } else {
                        BUS.publish(DomainEvent::DevicePeerOffline {
                            channel_id: status.channel_id,
                        });
                    }
                }
                Err(e) => {
                    log::warn!("[socket] failed to parse tunnel:peer-status: {e}");
                }
            }
        }
        // Device tunnel — encrypted frame from the iOS device.
        "tunnel:frame" => {
            log::debug!("[socket] tunnel:frame received");
            match serde_json::from_value::<crate::security::devices::tunnel_client::TunnelFrame>(
                data.clone(),
            ) {
                Ok(frame) => {
                    BUS.publish(DomainEvent::DeviceTunnelFrame {
                        channel_id: frame.channel_id,
                        payload_b64: frame.payload,
                    });
                }
                Err(e) => {
                    log::warn!("[socket] failed to parse tunnel:frame: {e}");
                }
            }
        }
        // Device tunnel — backend evicted the channel (TTL / server restart).
        "tunnel:evicted" => {
            let channel_id = data
                .get("channelId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            log::info!("[socket] tunnel:evicted channel_id={}", channel_id);
            if !channel_id.is_empty() {
                BUS.publish(DomainEvent::DevicePeerOffline { channel_id });
            }
        }

        // ── Backend Meet Bot events ──────────────────────────────────────
        "voice:harness" => {
            // Realtime voice-agent turn relayed from the backend Custom-LLM
            // bridge (#5399): run the local orchestrator and stream the reply
            // back up as voice:harness:delta / :done / :error.
            let correlation_id = data
                .get("correlationId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let messages: Vec<serde_json::Value> = data
                .get("messages")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            log::info!(
                "[socket] voice:harness correlation={} messages={}",
                correlation_id,
                messages.len()
            );
            if correlation_id.is_empty() {
                log::warn!("[socket] voice:harness missing correlationId — dropping");
            } else {
                #[cfg(feature = "voice")]
                tokio::spawn(crate::voice::realtime_harness::handle_voice_harness_turn(
                    // A relay socket event carries no context of its own.
                    crate::core::runtime::DispatchCtx::host_default(),
                    correlation_id,
                    messages,
                ));
                #[cfg(not(feature = "voice"))]
                {
                    let _ = messages;
                    log::warn!("[socket] voice:harness ignored — voice feature disabled");
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

            // Lift sender / reply_target / thread_ts off the raw payload so
            // the agent loop can derive per-sender conversation keys
            // instead of collapsing every inbound message in a shared
            // channel onto the same `channel:<name>` thread (which lets
            // one participant resume another's cached agent session).
            let nonempty = |v: Option<&serde_json::Value>| -> Option<String> {
                v.and_then(|x| x.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            let sender = nonempty(data.get("sender"))
                .or_else(|| nonempty(data.get("from")))
                .or_else(|| nonempty(data.get("user_id")));
            let reply_target = nonempty(data.get("reply_target"))
                .or_else(|| nonempty(data.get("chat_id")))
                .or_else(|| nonempty(data.get("channel_id")));
            let thread_ts =
                nonempty(data.get("thread_ts")).or_else(|| nonempty(data.get("thread_id")));

            BUS.publish(DomainEvent::ChannelInboundMessage {
                event_name: event_name.to_string(),
                channel,
                message,
                sender,
                reply_target,
                thread_ts,
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

#[cfg(test)]
#[path = "event_handlers_tests.rs"]
mod tests;
