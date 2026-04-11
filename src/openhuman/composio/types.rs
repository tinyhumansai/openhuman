//! Domain types for the Composio integration.
//!
//! These mirror the response envelopes emitted by the openhuman backend under
//! `/agent-integrations/composio/*`. See:
//!   - `src/routes/agentIntegrations/composio.ts`
//!   - `src/controllers/agentIntegrations/composio/*.ts`
//! in the backend repo for the authoritative shapes.

use serde::{Deserialize, Serialize};

// ── Toolkits ────────────────────────────────────────────────────────

/// Response body of `GET /agent-integrations/composio/toolkits`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposioToolkitsResponse {
    /// Server-enforced toolkit allowlist, e.g. `["gmail", "notion"]`.
    #[serde(default)]
    pub toolkits: Vec<String>,
}

// ── Connections ─────────────────────────────────────────────────────

/// One connected Composio account (OAuth integration instance).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioConnection {
    /// Composio connection id (what you DELETE to disconnect).
    pub id: String,
    /// Toolkit slug, e.g. `"gmail"`.
    pub toolkit: String,
    /// Connection status — `"ACTIVE"`, `"CONNECTED"`, `"PENDING"`, …
    pub status: String,
    /// ISO timestamp (backend passes this through from Composio).
    #[serde(rename = "createdAt", default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
}

/// Response body of `GET /agent-integrations/composio/connections`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposioConnectionsResponse {
    #[serde(default)]
    pub connections: Vec<ComposioConnection>,
}

/// Response body of `POST /agent-integrations/composio/authorize`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioAuthorizeResponse {
    /// Composio-hosted OAuth URL the user opens in a browser.
    #[serde(rename = "connectUrl")]
    pub connect_url: String,
    /// Composio connection id created by this authorize call.
    #[serde(rename = "connectionId")]
    pub connection_id: String,
}

/// Response body of `DELETE /agent-integrations/composio/connections/:id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioDeleteResponse {
    #[serde(default)]
    pub deleted: bool,
}

// ── Tools ───────────────────────────────────────────────────────────

/// OpenAI function-calling schema returned by the backend for each tool.
///
/// The backend wraps Composio's upstream shape; we keep the `type` +
/// `function` envelope so callers can forward directly into an LLM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioToolSchema {
    #[serde(rename = "type", default = "default_function_type")]
    pub kind: String,
    pub function: ComposioToolFunction,
}

fn default_function_type() -> String {
    "function".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioToolFunction {
    /// Composio action slug, e.g. `"GMAIL_SEND_EMAIL"`.
    pub name: String,
    /// Human-readable description shown to the model.
    #[serde(default)]
    pub description: Option<String>,
    /// JSON schema for the tool parameters.
    #[serde(default)]
    pub parameters: Option<serde_json::Value>,
}

/// Response body of `GET /agent-integrations/composio/tools`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposioToolsResponse {
    #[serde(default)]
    pub tools: Vec<ComposioToolSchema>,
}

// ── Execute ─────────────────────────────────────────────────────────

/// Response body of `POST /agent-integrations/composio/execute`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioExecuteResponse {
    /// Raw result from the upstream provider.
    #[serde(default)]
    pub data: serde_json::Value,
    /// Did the provider report success?
    #[serde(default)]
    pub successful: bool,
    /// Provider error message if any.
    #[serde(default)]
    pub error: Option<String>,
    /// Amount charged to the caller (base + margin) in USD.
    #[serde(rename = "costUsd", default)]
    pub cost_usd: f64,
}

// ── Triggers ────────────────────────────────────────────────────────

/// Payload of the `composio:trigger` Socket.IO event emitted by the backend
/// when a Composio webhook is received, HMAC-verified, and delivered to the
/// user's active sockets.
///
/// See `src/controllers/agentIntegrations/composio/handleWebhook.ts` in the
/// backend repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioTriggerEvent {
    /// Toolkit slug, e.g. `"gmail"`.
    #[serde(default)]
    pub toolkit: String,
    /// Trigger slug, e.g. `"GMAIL_NEW_GMAIL_MESSAGE"`.
    #[serde(default)]
    pub trigger: String,
    /// Trigger-specific payload (provider-defined shape).
    #[serde(default)]
    pub payload: serde_json::Value,
    /// Metadata the backend attaches: `{ id, uuid }`.
    #[serde(default)]
    pub metadata: ComposioTriggerMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ComposioTriggerMetadata {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub uuid: String,
}
