//! Event bus handlers for the webhook domain.
//!
//! The [`WebhookRequestSubscriber`] handles incoming webhook requests published
//! by the socket transport layer. It routes each request to the owning skill (or
//! echo target), waits for the response, and emits it back through the socket.
//! This decouples the socket module from webhook routing logic.

use crate::core::event_bus::{publish_global, DomainEvent, EventHandler};
use crate::openhuman::socket::global_socket_manager;
use crate::openhuman::webhooks::WebhookResponseData;
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
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

        // Retrieve the router from the global socket manager.
        let router = global_socket_manager().and_then(|mgr| mgr.webhook_router());

        // Look up the registration for this tunnel.
        let registration = router.as_ref().and_then(|r| r.registration(&tunnel_uuid));

        let (response, resolved_skill_id, response_error) = match registration {
            Some(ref reg) if reg.target_kind == "echo" => {
                tracing::debug!(
                    "[webhook] echo tunnel {} — returning echo response",
                    tunnel_uuid
                );
                let resp = crate::openhuman::webhooks::ops::build_echo_response(request);
                (resp, Some("echo".to_string()), None)
            }
            Some(ref reg) if reg.target_kind == "agent" => {
                tracing::info!(
                    "[webhook] agent tunnel {} — routing to triage pipeline",
                    tunnel_uuid
                );
                let decoded = decode_webhook_body(&request.body);
                if let Err(e) = &decoded {
                    tracing::error!("[webhook] rejecting — failed to decode body: {}", e);
                    let resp = WebhookResponseData {
                        correlation_id: correlation_id.clone(),
                        status_code: 400,
                        headers: HashMap::new(),
                        body: error_body(&format!("Invalid request body: {e}")),
                    };
                    (resp, None, Some(e.to_string()))
                } else {
                    let payload = decoded.unwrap();
                    let envelope = crate::openhuman::agent::triage::TriggerEnvelope::from_webhook(
                        &tunnel_uuid,
                        &method,
                        &path,
                        payload,
                    );
                    // Spawn the triage pipeline so we don't block the
                    // broadcast channel's dispatch task during LLM calls.
                    let corr = correlation_id.clone();
                    tokio::spawn(async move {
                        let result =
                            tokio::time::timeout(std::time::Duration::from_secs(60), async {
                                run_agent_trigger(&envelope).await
                            })
                            .await;
                        let (resp, err) = match result {
                            Ok(Ok(output)) => (build_agent_response(&corr, 200, &output), None),
                            Ok(Err(e)) => {
                                tracing::error!("[webhook] agent trigger failed: {}", e);
                                (
                                    build_agent_response(&corr, 500, &format!("Agent error: {e}")),
                                    Some(e),
                                )
                            }
                            Err(_) => {
                                tracing::error!("[webhook] agent trigger timed out (60s)");
                                (
                                    build_agent_response(&corr, 504, "Agent triage timed out"),
                                    Some("timed out after 60s".to_string()),
                                )
                            }
                        };
                        // Emit response from the spawned task.
                        if let Some(mgr) = global_socket_manager() {
                            let response_data = serde_json::json!({
                                "correlationId": resp.correlation_id,
                                "statusCode": resp.status_code,
                                "headers": resp.headers,
                                "body": resp.body,
                            });
                            if let Err(e) = mgr.emit("webhook:response", response_data).await {
                                tracing::error!("[webhook] failed to emit spawned response: {}", e);
                            }
                        }
                        if let Some(e) = err {
                            tracing::warn!("[webhook] agent trigger error: {}", e);
                        }
                    });
                    // Return 202 Accepted immediately so the event handler
                    // doesn't block the broadcast channel.
                    let resp = WebhookResponseData {
                        correlation_id: correlation_id.clone(),
                        status_code: 202,
                        headers: HashMap::new(),
                        body: serde_json::json!({"status": "accepted", "message": "Agent triage started"}).to_string(),
                    };
                    let skill_id = reg.agent_id.clone().or_else(|| Some(reg.skill_id.clone()));
                    (resp, skill_id, None)
                }
            }
            Some(ref reg) => {
                // skill target kind or any other unrecognised kind — skill runtime not available
                tracing::debug!(
                    "[webhook] skill tunnel {} (kind={}) — skill runtime not available",
                    tunnel_uuid,
                    reg.target_kind,
                );
                let resp = WebhookResponseData {
                    correlation_id: correlation_id.clone(),
                    status_code: 501,
                    headers: HashMap::new(),
                    body: error_body("Skill runtime not available for direct dispatch"),
                };
                (
                    resp,
                    Some(reg.skill_id.clone()),
                    Some("skill runtime not available".to_string()),
                )
            }
            None => {
                tracing::debug!("[webhook] no registration for tunnel {}", tunnel_uuid);
                let resp = WebhookResponseData {
                    correlation_id: correlation_id.clone(),
                    status_code: 404,
                    headers: HashMap::new(),
                    body: error_body("No tunnel registration found"),
                };
                (resp, None, Some("no tunnel registration".to_string()))
            }
        };

        // Record request and response in the router debug logs.
        if let Some(ref r) = router {
            r.record_request(request, resolved_skill_id.clone());
            r.record_response(
                request,
                &response,
                resolved_skill_id.clone(),
                response_error.clone(),
            );
        }

        // Publish notification events.
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

        // Emit response back through the socket.
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
            "[webhook] {} {} → status={}, skill={:?}, tunnel={} ({}ms)",
            method,
            path,
            response.status_code,
            resolved_skill_id,
            tunnel_name,
            started_at.elapsed().as_millis(),
        );
    }
}

/// Decode a base64-encoded webhook request body into a JSON value.
///
/// Returns an empty object when the body is absent, empty, or not valid
/// UTF-8 JSON. If the body is valid UTF-8 but not valid JSON, the raw
/// text is wrapped under the `"raw"` key so callers still have access
/// to the original content.
fn decode_webhook_body(base64_body: &str) -> Result<serde_json::Value, String> {
    if base64_body.is_empty() {
        return Ok(serde_json::json!({}));
    }
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(base64_body.as_bytes())
        .map_err(|e| format!("invalid base64 body: {e}"))?;
    let text = std::str::from_utf8(&decoded).map_err(|e| format!("invalid utf-8 body: {e}"))?;
    Ok(serde_json::from_str(text).unwrap_or_else(|_| serde_json::json!({ "raw": text })))
}

/// Run the triage pipeline for a trigger envelope and return the
/// human-readable decision summary on success.
async fn run_agent_trigger(
    envelope: &crate::openhuman::agent::triage::TriggerEnvelope,
) -> Result<String, String> {
    let run = crate::openhuman::agent::triage::run_triage(envelope)
        .await
        .map_err(|e| format!("triage evaluation failed: {e}"))?;

    crate::openhuman::agent::triage::apply_decision(run.clone(), envelope)
        .await
        .map_err(|e| format!("apply_decision failed: {e}"))?;

    Ok(format!(
        "Triage decision: {} (agent: {:?})",
        run.decision.action.as_str(),
        run.decision.target_agent
    ))
}

/// Build a base64-encoded JSON response body for an agent trigger result.
fn build_agent_response(
    correlation_id: &str,
    status_code: u16,
    body_text: &str,
) -> WebhookResponseData {
    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    WebhookResponseData {
        correlation_id: correlation_id.to_string(),
        status_code,
        headers,
        body: base64_encode(&serde_json::json!({ "result": body_text }).to_string()),
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
        // When the socket-manager singleton isn't initialised, the router
        // lookup returns None (no registration), so the handler takes the
        // "no tunnel registration → 404" path and then logs "no socket
        // manager available" before returning cleanly.
        let subscriber = WebhookRequestSubscriber::new();
        let request = WebhookRequest {
            correlation_id: "wh_test_1".into(),
            tunnel_id: "tid-1".into(),
            tunnel_uuid: "uuid-unregistered".into(),
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
        // Must not panic — even without any singletons initialised.
        subscriber.handle(&event).await;
    }

    // ── decode_webhook_body ───────────────────────────────────────

    #[test]
    fn decode_webhook_body_empty_returns_empty_object() {
        let v = decode_webhook_body("").unwrap();
        assert!(v.as_object().map(|o| o.is_empty()).unwrap_or(false));
    }

    #[test]
    fn decode_webhook_body_parses_valid_json() {
        use base64::Engine;
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(r#"{"key":"value"}"#.as_bytes());
        let v = decode_webhook_body(&encoded).unwrap();
        assert_eq!(v["key"].as_str(), Some("value"));
    }

    #[test]
    fn decode_webhook_body_wraps_non_json_in_raw_field() {
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode("plain text".as_bytes());
        let v = decode_webhook_body(&encoded).unwrap();
        assert_eq!(v["raw"].as_str(), Some("plain text"));
    }

    #[test]
    fn decode_webhook_body_rejects_invalid_base64() {
        let err = decode_webhook_body("not-valid-base64!!!").unwrap_err();
        assert!(err.contains("invalid base64"));
    }

    // ── build_agent_response ──────────────────────────────────────

    #[test]
    fn build_agent_response_sets_status_and_body() {
        let resp = build_agent_response("corr-1", 200, "Triage decision: drop");
        assert_eq!(resp.correlation_id, "corr-1");
        assert_eq!(resp.status_code, 200);
        assert_eq!(
            resp.headers.get("content-type").map(String::as_str),
            Some("application/json")
        );
        // Body must be base64-encoded JSON with a "result" key.
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(resp.body.as_bytes())
            .expect("valid base64");
        let v: serde_json::Value = serde_json::from_slice(&decoded).expect("valid json");
        assert_eq!(v["result"].as_str(), Some("Triage decision: drop"));
    }
}
