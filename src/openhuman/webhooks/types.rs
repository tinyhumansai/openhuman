//! Core types for webhook tunnel routing.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Incoming webhook request forwarded from the backend via Socket.IO.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookRequest {
    /// Correlation ID for request-response matching (e.g. `wh_uuid_ts_hex`).
    #[serde(rename = "correlationId")]
    pub correlation_id: String,
    /// Backend tunnel ID.
    #[serde(rename = "tunnelId")]
    pub tunnel_id: String,
    /// Tunnel UUID (used for routing to the owning skill).
    #[serde(rename = "tunnelUuid")]
    pub tunnel_uuid: String,
    /// Human-readable tunnel name.
    #[serde(rename = "tunnelName")]
    pub tunnel_name: String,
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// Request path after the tunnel prefix.
    pub path: String,
    /// Request headers.
    pub headers: HashMap<String, serde_json::Value>,
    /// Query string parameters.
    pub query: HashMap<String, String>,
    /// Base64-encoded request body.
    #[serde(default)]
    pub body: String,
}

/// Response data sent back to the backend for a webhook request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookResponseData {
    /// Must match the incoming request's correlation_id.
    #[serde(rename = "correlationId")]
    pub correlation_id: String,
    /// HTTP status code to return.
    #[serde(rename = "statusCode")]
    pub status_code: u16,
    /// Response headers.
    #[serde(default)]
    pub headers: HashMap<String, String>,
    /// Base64-encoded response body.
    #[serde(default)]
    pub body: String,
}

/// A mapping from a tunnel UUID to the skill that owns it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelRegistration {
    /// Tunnel UUID (from the backend).
    pub tunnel_uuid: String,
    /// Registration target kind (`skill`, `channel`, or `echo`).
    #[serde(default = "default_webhook_target_kind")]
    pub target_kind: String,
    /// Skill ID that owns and handles this tunnel.
    pub skill_id: String,
    /// Human-readable tunnel name (optional, for display).
    #[serde(default)]
    pub tunnel_name: Option<String>,
    /// Backend MongoDB `_id` for CRUD operations.
    #[serde(default)]
    pub backend_tunnel_id: Option<String>,
}

fn default_webhook_target_kind() -> String {
    "skill".to_string()
}

/// Entry in the webhook activity log, emitted to the frontend via Tauri events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookActivityEntry {
    /// Correlation ID of the request.
    pub correlation_id: String,
    /// Tunnel name.
    pub tunnel_name: String,
    /// HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// Response status code (None if timed out or no handler).
    pub status_code: Option<u16>,
    /// Skill that handled the request (None if unrouted).
    pub skill_id: Option<String>,
    /// Unix timestamp in milliseconds.
    pub timestamp: u64,
}

/// Full webhook debug log entry retained for developer inspection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDebugLogEntry {
    /// Correlation ID of the request.
    pub correlation_id: String,
    /// Backend tunnel ID.
    pub tunnel_id: String,
    /// Tunnel UUID.
    pub tunnel_uuid: String,
    /// Tunnel name.
    pub tunnel_name: String,
    /// HTTP method.
    pub method: String,
    /// Request path.
    pub path: String,
    /// Owning skill if known.
    pub skill_id: Option<String>,
    /// Most recent response status code, if available.
    pub status_code: Option<u16>,
    /// Unix timestamp in milliseconds when the request was first seen.
    pub timestamp: u64,
    /// Unix timestamp in milliseconds for the latest update.
    pub updated_at: u64,
    /// Request headers as forwarded from the backend.
    #[serde(default)]
    pub request_headers: HashMap<String, serde_json::Value>,
    /// Query parameters.
    #[serde(default)]
    pub request_query: HashMap<String, String>,
    /// Base64-encoded request body.
    #[serde(default)]
    pub request_body: String,
    /// Response headers returned by the skill/core.
    #[serde(default)]
    pub response_headers: HashMap<String, String>,
    /// Base64-encoded response body.
    #[serde(default)]
    pub response_body: String,
    /// Current lifecycle stage.
    pub stage: String,
    /// Error detail when capture or routing failed.
    pub error_message: Option<String>,
    /// Raw payload snapshot for malformed webhook events.
    pub raw_payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDebugRegistrationsResult {
    pub registrations: Vec<TunnelRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDebugLogListResult {
    pub logs: Vec<WebhookDebugLogEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDebugLogsClearedResult {
    pub cleared: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebhookDebugEvent {
    pub event_type: String,
    pub timestamp: u64,
    pub correlation_id: Option<String>,
    pub tunnel_uuid: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── WebhookRequest ─────────────────────────────────────────────

    #[test]
    fn webhook_request_deserializes_camel_case_ids_and_defaults_body() {
        // Body is `#[serde(default)]` — missing body must deserialise
        // to the empty string rather than erroring.
        let payload = json!({
            "correlationId": "wh_abc_123",
            "tunnelId": "tid-1",
            "tunnelUuid": "uuid-1",
            "tunnelName": "my-hook",
            "method": "POST",
            "path": "/x",
            "headers": {"X-Foo": "bar"},
            "query": {"q": "1"}
        });
        let req: WebhookRequest = serde_json::from_value(payload).unwrap();
        assert_eq!(req.correlation_id, "wh_abc_123");
        assert_eq!(req.tunnel_id, "tid-1");
        assert_eq!(req.tunnel_uuid, "uuid-1");
        assert_eq!(req.tunnel_name, "my-hook");
        assert_eq!(req.method, "POST");
        assert_eq!(req.path, "/x");
        assert_eq!(req.headers.get("X-Foo"), Some(&json!("bar")));
        assert_eq!(req.query.get("q").map(String::as_str), Some("1"));
        assert_eq!(req.body, "");
    }

    #[test]
    fn webhook_request_serializes_back_to_camel_case_keys() {
        let req = WebhookRequest {
            correlation_id: "c".into(),
            tunnel_id: "t".into(),
            tunnel_uuid: "u".into(),
            tunnel_name: "n".into(),
            method: "GET".into(),
            path: "/".into(),
            headers: HashMap::new(),
            query: HashMap::new(),
            body: "aGVsbG8=".into(),
        };
        let v = serde_json::to_value(&req).unwrap();
        assert!(v.get("correlationId").is_some());
        assert!(v.get("tunnelId").is_some());
        assert!(v.get("tunnelUuid").is_some());
        assert!(v.get("tunnelName").is_some());
        assert_eq!(v.get("body").and_then(|b| b.as_str()), Some("aGVsbG8="));
    }

    // ── WebhookResponseData ────────────────────────────────────────

    #[test]
    fn webhook_response_data_defaults_headers_and_body() {
        let payload = json!({
            "correlationId": "c",
            "statusCode": 204
        });
        let resp: WebhookResponseData = serde_json::from_value(payload).unwrap();
        assert_eq!(resp.correlation_id, "c");
        assert_eq!(resp.status_code, 204);
        assert!(resp.headers.is_empty());
        assert_eq!(resp.body, "");
    }

    #[test]
    fn webhook_response_data_round_trips() {
        let resp = WebhookResponseData {
            correlation_id: "c".into(),
            status_code: 200,
            headers: [("Content-Type".to_string(), "text/plain".to_string())]
                .into_iter()
                .collect(),
            body: "Zm9v".into(),
        };
        let s = serde_json::to_string(&resp).unwrap();
        let back: WebhookResponseData = serde_json::from_str(&s).unwrap();
        assert_eq!(back.status_code, 200);
        assert_eq!(back.headers.get("Content-Type").map(String::as_str), Some("text/plain"));
        assert_eq!(back.body, "Zm9v");
    }

    // ── TunnelRegistration + default_webhook_target_kind ──────────

    #[test]
    fn default_webhook_target_kind_is_skill() {
        assert_eq!(default_webhook_target_kind(), "skill");
    }

    #[test]
    fn tunnel_registration_defaults_target_kind_to_skill() {
        // Omitting `target_kind` must fall back to "skill" via the
        // `#[serde(default = "default_webhook_target_kind")]` attribute.
        let payload = json!({
            "tunnel_uuid": "u-1",
            "skill_id": "gmail"
        });
        let reg: TunnelRegistration = serde_json::from_value(payload).unwrap();
        assert_eq!(reg.tunnel_uuid, "u-1");
        assert_eq!(reg.target_kind, "skill");
        assert_eq!(reg.skill_id, "gmail");
        assert!(reg.tunnel_name.is_none());
        assert!(reg.backend_tunnel_id.is_none());
    }

    #[test]
    fn tunnel_registration_honours_explicit_target_kind() {
        let payload = json!({
            "tunnel_uuid": "u-1",
            "target_kind": "echo",
            "skill_id": "echo",
            "tunnel_name": "my",
            "backend_tunnel_id": "b-1"
        });
        let reg: TunnelRegistration = serde_json::from_value(payload).unwrap();
        assert_eq!(reg.target_kind, "echo");
        assert_eq!(reg.tunnel_name.as_deref(), Some("my"));
        assert_eq!(reg.backend_tunnel_id.as_deref(), Some("b-1"));
    }

    // ── WebhookActivityEntry ──────────────────────────────────────

    #[test]
    fn webhook_activity_entry_round_trips_optional_fields() {
        let entry = WebhookActivityEntry {
            correlation_id: "c".into(),
            tunnel_name: "t".into(),
            method: "POST".into(),
            path: "/p".into(),
            status_code: Some(200),
            skill_id: Some("gmail".into()),
            timestamp: 1_700_000_000_000,
        };
        let s = serde_json::to_string(&entry).unwrap();
        let back: WebhookActivityEntry = serde_json::from_str(&s).unwrap();
        assert_eq!(back.status_code, Some(200));
        assert_eq!(back.skill_id.as_deref(), Some("gmail"));

        let unrouted = WebhookActivityEntry {
            status_code: None,
            skill_id: None,
            ..entry
        };
        let s2 = serde_json::to_string(&unrouted).unwrap();
        let back2: WebhookActivityEntry = serde_json::from_str(&s2).unwrap();
        assert!(back2.status_code.is_none());
        assert!(back2.skill_id.is_none());
    }

    // ── WebhookDebugLogEntry ──────────────────────────────────────

    #[test]
    fn webhook_debug_log_entry_defaults_request_response_payloads() {
        // Five `#[serde(default)]` fields — omit them all in the JSON
        // and confirm they come back as empty collections / strings.
        let payload = json!({
            "correlation_id": "c",
            "tunnel_id": "t",
            "tunnel_uuid": "u",
            "tunnel_name": "n",
            "method": "GET",
            "path": "/",
            "skill_id": null,
            "status_code": null,
            "timestamp": 1,
            "updated_at": 2,
            "stage": "received",
            "error_message": null,
            "raw_payload": null
        });
        let entry: WebhookDebugLogEntry = serde_json::from_value(payload).unwrap();
        assert!(entry.request_headers.is_empty());
        assert!(entry.request_query.is_empty());
        assert_eq!(entry.request_body, "");
        assert!(entry.response_headers.is_empty());
        assert_eq!(entry.response_body, "");
        assert_eq!(entry.timestamp, 1);
        assert_eq!(entry.updated_at, 2);
    }

    // ── Debug* result wrappers ────────────────────────────────────

    #[test]
    fn debug_result_wrappers_round_trip() {
        let regs = WebhookDebugRegistrationsResult {
            registrations: vec![TunnelRegistration {
                tunnel_uuid: "u".into(),
                target_kind: "skill".into(),
                skill_id: "s".into(),
                tunnel_name: None,
                backend_tunnel_id: None,
            }],
        };
        let back: WebhookDebugRegistrationsResult =
            serde_json::from_str(&serde_json::to_string(&regs).unwrap()).unwrap();
        assert_eq!(back.registrations.len(), 1);

        let logs = WebhookDebugLogListResult { logs: vec![] };
        let back: WebhookDebugLogListResult =
            serde_json::from_str(&serde_json::to_string(&logs).unwrap()).unwrap();
        assert!(back.logs.is_empty());

        let cleared = WebhookDebugLogsClearedResult { cleared: 7 };
        let back: WebhookDebugLogsClearedResult =
            serde_json::from_str(&serde_json::to_string(&cleared).unwrap()).unwrap();
        assert_eq!(back.cleared, 7);
    }

    // ── WebhookDebugEvent ─────────────────────────────────────────

    #[test]
    fn webhook_debug_event_round_trips_optional_correlation_fields() {
        let ev = WebhookDebugEvent {
            event_type: "request".into(),
            timestamp: 123,
            correlation_id: Some("c".into()),
            tunnel_uuid: Some("u".into()),
        };
        let s = serde_json::to_string(&ev).unwrap();
        let back: WebhookDebugEvent = serde_json::from_str(&s).unwrap();
        assert_eq!(back.event_type, "request");
        assert_eq!(back.timestamp, 123);
        assert_eq!(back.correlation_id.as_deref(), Some("c"));
        assert_eq!(back.tunnel_uuid.as_deref(), Some("u"));
    }
}
