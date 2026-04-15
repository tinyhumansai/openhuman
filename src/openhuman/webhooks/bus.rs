//! Event bus handlers for the webhook domain.
//!
//! The [`WebhookRequestSubscriber`] handles incoming webhook requests published
//! by the socket transport layer. It routes each request to the owning skill (or
//! echo target), waits for the response, and emits it back through the socket.
//! This decouples the socket module from webhook routing logic.

use crate::core::event_bus::{publish_global, DomainEvent, EventHandler};
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

        tracing::debug!(
            "[webhook] skill runtime removed; rejecting tunnel {} ({})",
            tunnel_uuid,
            tunnel_name
        );
        let response = crate::openhuman::webhooks::WebhookResponseData {
            correlation_id: correlation_id.clone(),
            status_code: 410,
            headers: std::collections::HashMap::new(),
            body: error_body("Webhook skill runtime has been removed"),
        };
        let resolved_skill_id: Option<String> = None;
        let response_error = Some("webhook skill runtime removed".to_string());

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::webhooks::WebhookRequest;
    use base64::Engine;
    use std::collections::HashMap;

    // ── Local helpers ─────────────────────────────────────────────

    #[test]
    fn base64_encode_matches_standard_engine_output() {
        assert_eq!(base64_encode("hello"), "aGVsbG8=");
        assert_eq!(base64_encode(""), "");
    }

    #[test]
    fn error_body_is_base64_of_json_envelope() {
        let encoded = error_body("boom");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .expect("valid base64");
        let json: serde_json::Value = serde_json::from_slice(&decoded).expect("valid json");
        assert_eq!(json["error"].as_str(), Some("boom"));
    }

    // ── Constructor + EventHandler metadata ───────────────────────

    #[test]
    fn default_equals_new_and_is_zero_sized() {
        // Both constructors produce the same unit-variant struct.
        let _a = WebhookRequestSubscriber::default();
        let _b = WebhookRequestSubscriber::new();
        // Zero-sized type — just asserting both compile and construct.
        assert_eq!(std::mem::size_of::<WebhookRequestSubscriber>(), 0);
    }

    #[test]
    fn event_handler_name_is_namespaced() {
        let s = WebhookRequestSubscriber::new();
        assert_eq!(s.name(), "webhook::request_handler");
    }

    #[test]
    fn event_handler_domain_filter_is_webhook() {
        let s = WebhookRequestSubscriber::new();
        assert_eq!(s.domains(), Some(&["webhook"][..]));
    }

    // ── handle() behaviour ────────────────────────────────────────

    #[tokio::test]
    async fn handle_returns_early_on_non_webhook_event() {
        // A domain event for a different module must be ignored —
        // `handle()` checks the variant and returns without touching
        // the socket manager or publishing anything.
        let subscriber = WebhookRequestSubscriber::new();
        let event = DomainEvent::AgentTurnStarted {
            session_id: "s1".into(),
            channel: "web".into(),
        };
        // Must not panic, must not block — even without any singletons
        // initialised in the test process.
        subscriber.handle(&event).await;
    }

    #[tokio::test]
    async fn handle_processes_incoming_webhook_without_socket_manager() {
        // When the socket-manager singleton isn't initialised, the
        // handler should log "no socket manager available" and return
        // cleanly rather than panicking. We exercise the full routing
        // path (currently the "skill runtime removed" branch) to lock
        // in that contract for future refactors.
        let subscriber = WebhookRequestSubscriber::new();
        let request = WebhookRequest {
            correlation_id: "wh_test_1".into(),
            tunnel_id: "tid-1".into(),
            tunnel_uuid: "uuid-1".into(),
            tunnel_name: "my-hook".into(),
            method: "POST".into(),
            path: "/hook".into(),
            headers: HashMap::new(),
            query: HashMap::new(),
            body: String::new(),
        };
        let event = DomainEvent::WebhookIncomingRequest {
            request,
            raw_data: serde_json::json!({}),
        };
        subscriber.handle(&event).await;
    }
}
