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
    /// Skill ID that owns and handles this tunnel.
    pub skill_id: String,
    /// Human-readable tunnel name (optional, for display).
    #[serde(default)]
    pub tunnel_name: Option<String>,
    /// Backend MongoDB `_id` for CRUD operations.
    #[serde(default)]
    pub backend_tunnel_id: Option<String>,
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
