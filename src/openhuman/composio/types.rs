//! Domain types for the Composio integration.
//!
//! These mirror the response envelopes emitted by the openhuman backend under
//! `/agent-integrations/composio/*`. See:
//!   - `src/routes/agentIntegrations/composio.ts`
//!   - `src/controllers/agentIntegrations/composio/*.ts`
//!     in the backend repo for the authoritative shapes.

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

// ── GitHub repos + triggers ─────────────────────────────────────────

/// One repository returned by `GET /agent-integrations/composio/github/repos`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioGithubRepo {
    pub owner: String,
    pub repo: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    #[serde(default)]
    pub private: Option<bool>,
    #[serde(rename = "defaultBranch", default)]
    pub default_branch: Option<String>,
    #[serde(rename = "htmlUrl", default)]
    pub html_url: Option<String>,
}

/// Response body of `GET /agent-integrations/composio/github/repos`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioGithubReposResponse {
    #[serde(rename = "connectionId")]
    pub connection_id: String,
    #[serde(default, rename = "repositories")]
    pub repositories: Vec<ComposioGithubRepo>,
}

/// Response body of `POST /agent-integrations/composio/triggers`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioCreateTriggerResponse {
    #[serde(rename = "triggerId")]
    pub trigger_id: String,
    #[serde(default)]
    pub status: Option<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioTriggerHistoryEntry {
    /// Unix timestamp in milliseconds when the trigger reached the core.
    pub received_at_ms: u64,
    /// Toolkit slug, e.g. `"gmail"`.
    pub toolkit: String,
    /// Trigger slug, e.g. `"GMAIL_NEW_GMAIL_MESSAGE"`.
    pub trigger: String,
    /// Backend metadata id for this event.
    pub metadata_id: String,
    /// Backend metadata UUID for this event.
    pub metadata_uuid: String,
    /// Raw provider payload as forwarded by the backend socket event.
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComposioTriggerHistoryResult {
    /// Directory containing daily JSONL archives.
    pub archive_dir: String,
    /// Today's JSONL file path.
    pub current_day_file: String,
    /// Recent triggers, newest first.
    pub entries: Vec<ComposioTriggerHistoryEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn toolkits_response_defaults_to_empty() {
        let resp: ComposioToolkitsResponse = serde_json::from_str("{}").unwrap();
        assert!(resp.toolkits.is_empty());
    }

    #[test]
    fn toolkits_response_roundtrips() {
        let resp = ComposioToolkitsResponse {
            toolkits: vec!["gmail".into(), "notion".into()],
        };
        let value = serde_json::to_value(&resp).unwrap();
        assert_eq!(value, json!({ "toolkits": ["gmail", "notion"] }));
        let back: ComposioToolkitsResponse = serde_json::from_value(value).unwrap();
        assert_eq!(back.toolkits, vec!["gmail", "notion"]);
    }

    #[test]
    fn connection_parses_and_serializes_camelcase_created_at() {
        let raw = json!({
            "id": "conn_1",
            "toolkit": "gmail",
            "status": "ACTIVE",
            "createdAt": "2026-02-01T00:00:00Z"
        });
        let conn: ComposioConnection = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(conn.id, "conn_1");
        assert_eq!(conn.toolkit, "gmail");
        assert_eq!(conn.status, "ACTIVE");
        assert_eq!(conn.created_at.as_deref(), Some("2026-02-01T00:00:00Z"));

        // Round-trip must use camelCase too.
        let serialized = serde_json::to_value(&conn).unwrap();
        assert!(serialized.get("createdAt").is_some());
    }

    #[test]
    fn connection_without_created_at_omits_field_when_serialized() {
        let conn = ComposioConnection {
            id: "x".into(),
            toolkit: "notion".into(),
            status: "PENDING".into(),
            created_at: None,
        };
        let s = serde_json::to_value(&conn).unwrap();
        assert!(
            s.get("createdAt").is_none(),
            "createdAt must be skipped when None"
        );
    }

    #[test]
    fn authorize_response_uses_camelcase_keys() {
        let raw = json!({
            "connectUrl": "https://composio.dev/oauth/abc",
            "connectionId": "conn_2"
        });
        let resp: ComposioAuthorizeResponse = serde_json::from_value(raw).unwrap();
        assert_eq!(resp.connect_url, "https://composio.dev/oauth/abc");
        assert_eq!(resp.connection_id, "conn_2");

        let s = serde_json::to_value(&resp).unwrap();
        assert!(s.get("connectUrl").is_some());
        assert!(s.get("connectionId").is_some());
    }

    #[test]
    fn tool_schema_defaults_type_field_to_function() {
        let raw = json!({
            "function": {
                "name": "GMAIL_SEND_EMAIL",
                "description": "Send an email",
                "parameters": { "type": "object" }
            }
        });
        let tool: ComposioToolSchema = serde_json::from_value(raw).unwrap();
        assert_eq!(tool.kind, "function");
        assert_eq!(tool.function.name, "GMAIL_SEND_EMAIL");
        assert_eq!(tool.function.description.as_deref(), Some("Send an email"));
        assert!(tool.function.parameters.is_some());
    }

    #[test]
    fn tool_function_tolerates_missing_description_and_parameters() {
        let raw = json!({ "function": { "name": "SLUG_ONLY" } });
        let tool: ComposioToolSchema = serde_json::from_value(raw).unwrap();
        assert_eq!(tool.function.name, "SLUG_ONLY");
        assert!(tool.function.description.is_none());
        assert!(tool.function.parameters.is_none());
    }

    #[test]
    fn execute_response_parses_cost_and_error() {
        let raw = json!({
            "data": { "messageId": "m-1" },
            "successful": true,
            "error": null,
            "costUsd": 0.0025
        });
        let resp: ComposioExecuteResponse = serde_json::from_value(raw).unwrap();
        assert!(resp.successful);
        assert!(resp.error.is_none());
        assert!((resp.cost_usd - 0.0025).abs() < f64::EPSILON);
    }

    #[test]
    fn execute_response_defaults_when_fields_missing() {
        let resp: ComposioExecuteResponse = serde_json::from_str("{}").unwrap();
        assert!(!resp.successful);
        assert!(resp.error.is_none());
        assert_eq!(resp.cost_usd, 0.0);
        assert!(resp.data.is_null());
    }

    #[test]
    fn trigger_event_defaults_empty_fields_to_empty_strings() {
        let ev: ComposioTriggerEvent = serde_json::from_str("{}").unwrap();
        assert_eq!(ev.toolkit, "");
        assert_eq!(ev.trigger, "");
        assert_eq!(ev.metadata.id, "");
        assert_eq!(ev.metadata.uuid, "");
        assert!(ev.payload.is_null());
    }

    #[test]
    fn trigger_event_parses_full_payload() {
        let raw = json!({
            "toolkit": "gmail",
            "trigger": "GMAIL_NEW_GMAIL_MESSAGE",
            "payload": { "subject": "hi" },
            "metadata": { "id": "evt-1", "uuid": "uuid-1" }
        });
        let ev: ComposioTriggerEvent = serde_json::from_value(raw).unwrap();
        assert_eq!(ev.toolkit, "gmail");
        assert_eq!(ev.trigger, "GMAIL_NEW_GMAIL_MESSAGE");
        assert_eq!(ev.metadata.id, "evt-1");
        assert_eq!(ev.metadata.uuid, "uuid-1");
        assert_eq!(ev.payload["subject"], "hi");
    }
}
