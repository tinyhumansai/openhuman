//! Event bus handlers for the webhook domain.
//!
//! The [`WebhookRequestSubscriber`] handles incoming webhook requests published
//! by the socket transport layer. It routes each request to the owning skill (or
//! echo target), waits for the response, and emits it back through the socket.
//! This decouples the socket module from webhook routing logic.

use crate::openhuman::event_bus::{publish_global, DomainEvent, EventHandler};
use crate::openhuman::socket::global_socket_manager;
use async_trait::async_trait;
use serde_json::json;
use std::time::Instant;

/// Base64-encode a string (for webhook response bodies).
fn base64_encode(input: &str) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
}

/// Build a base64-encoded JSON error body using proper serialization.
fn error_body(message: &str) -> String {
    let obj = serde_json::json!({ "error": message });
    base64_encode(&obj.to_string())
}

/// Subscribes to `WebhookIncomingRequest` events and handles the full routing
/// flow: lookup tunnel → dispatch to skill/echo → emit response via socket.
pub struct WebhookRequestSubscriber;

impl Default for WebhookRequestSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl WebhookRequestSubscriber {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler for WebhookRequestSubscriber {
    fn name(&self) -> &str {
        "webhook::request_handler"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["webhook"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::WebhookIncomingRequest {
            request,
            raw_data: _,
        } = event
        else {
            return;
        };

        let started_at = Instant::now();

        let correlation_id = request.correlation_id.clone();
        let tunnel_uuid = request.tunnel_uuid.clone();
        let tunnel_name = request.tunnel_name.clone();
        let method = request.method.clone();
        let path = request.path.clone();

        tracing::info!(
            "[webhook] incoming request {} {} (tunnel={}, correlationId={})",
            method,
            path,
            tunnel_uuid,
            correlation_id,
        );

        // Get the webhook router from the skill engine
        let router = crate::openhuman::skills::global_engine().map(|e| e.webhook_router());

        let registration = router.as_ref().and_then(|r| r.registration(&tunnel_uuid));
        let skill_id = registration
            .as_ref()
            .and_then(|reg| (reg.target_kind == "skill").then(|| reg.skill_id.clone()));
        if let Some(ref router) = router {
            router.record_request(request, skill_id.clone());
        }

        let (response, resolved_skill_id, response_error) = match registration.as_ref() {
            Some(reg) if reg.target_kind == "echo" => (
                crate::openhuman::webhooks::ops::build_echo_response(request),
                Some("echo".to_string()),
                None,
            ),
            Some(reg) if reg.target_kind == "channel" => (
                crate::openhuman::webhooks::WebhookResponseData {
                    correlation_id: correlation_id.clone(),
                    status_code: 501,
                    headers: std::collections::HashMap::new(),
                    body: error_body(&format!(
                        "channel webhook target '{}' is not implemented in this runtime yet",
                        reg.skill_id
                    )),
                },
                Some(reg.skill_id.clone()),
                Some("channel webhook target not implemented".to_string()),
            ),
            Some(reg) if reg.target_kind == "skill" => {
                let sid = reg.skill_id.clone();
                tracing::debug!("[webhook] request routed to skill '{}'", sid);

                let registry = crate::openhuman::skills::global_engine().map(|e| e.registry());
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
                                tracing::warn!("[webhook] skill handler error: {}", e);
                                (
                                    crate::openhuman::webhooks::WebhookResponseData {
                                        correlation_id: correlation_id.clone(),
                                        status_code: 500,
                                        headers: std::collections::HashMap::new(),
                                        body: error_body(&format!("Skill handler error: {}", e)),
                                    },
                                    Some(sid),
                                    Some(e),
                                )
                            }
                        }
                    }
                    None => {
                        tracing::warn!("[webhook] no skill registry available");
                        (
                            crate::openhuman::webhooks::WebhookResponseData {
                                correlation_id: correlation_id.clone(),
                                status_code: 503,
                                headers: std::collections::HashMap::new(),
                                body: error_body("Runtime not ready"),
                            },
                            None,
                            Some("runtime not ready".to_string()),
                        )
                    }
                }
            }
            Some(reg) => (
                crate::openhuman::webhooks::WebhookResponseData {
                    correlation_id: correlation_id.clone(),
                    status_code: 400,
                    headers: std::collections::HashMap::new(),
                    body: error_body(&format!(
                        "unknown webhook target kind '{}'",
                        reg.target_kind
                    )),
                },
                Some(reg.skill_id.clone()),
                Some("unknown webhook target kind".to_string()),
            ),
            None => {
                tracing::debug!("[webhook] no handler registered for tunnel {}", tunnel_uuid,);
                (
                    crate::openhuman::webhooks::WebhookResponseData {
                        correlation_id: correlation_id.clone(),
                        status_code: 404,
                        headers: std::collections::HashMap::new(),
                        body: error_body("No handler registered for this tunnel"),
                    },
                    None,
                    Some("no handler registered for this tunnel".to_string()),
                )
            }
        };

        // Record in debug log
        if let Some(ref router) = router {
            router.record_response(
                request,
                &response,
                resolved_skill_id.clone(),
                response_error.clone(),
            );
        }

        // Publish notification events
        if let Some(ref sid) = resolved_skill_id {
            publish_global(DomainEvent::WebhookReceived {
                tunnel_id: tunnel_uuid.clone(),
                skill_id: sid.clone(),
                method: method.clone(),
                path: path.clone(),
                correlation_id: correlation_id.clone(),
            });
        }
        publish_global(DomainEvent::WebhookProcessed {
            tunnel_id: tunnel_uuid.clone(),
            skill_id: resolved_skill_id.clone().unwrap_or_default(),
            method: method.clone(),
            path: path.clone(),
            correlation_id: correlation_id.clone(),
            status_code: response.status_code,
            elapsed_ms: started_at.elapsed().as_millis() as u64,
            error: response_error.clone(),
        });

        // Emit response back through the socket
        if let Some(mgr) = global_socket_manager() {
            let response_data = json!({
                "correlationId": response.correlation_id,
                "statusCode": response.status_code,
                "headers": response.headers,
                "body": response.body,
            });
            if let Err(e) = mgr.emit("webhook:response", response_data).await {
                tracing::error!("[webhook] failed to emit response via socket: {}", e);
            }
        } else {
            tracing::error!("[webhook] no socket manager available to emit response");
        }

        tracing::info!(
            "[webhook] {} {} → status={}, skill={:?}, tunnel={}",
            method,
            path,
            response.status_code,
            resolved_skill_id,
            tunnel_name,
        );
    }
}
