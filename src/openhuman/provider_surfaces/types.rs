//! Shared types for provider assistive surfaces.

use serde::{Deserialize, Serialize};

/// Inbound normalized provider event suitable for local assistive handling.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ProviderEvent {
    pub provider: String,
    pub account_id: String,
    pub event_kind: String,
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_handle: Option<String>,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    #[serde(default)]
    pub requires_attention: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_payload: Option<serde_json::Value>,
}

/// Queue item shown in the local respond queue.
///
/// Field naming mirrors `ProviderEvent` and the declared controller schema
/// (`provider_surfaces::ingest_event` inputs), so callers see a single
/// snake_case contract on both request and response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RespondQueueItem {
    pub id: String,
    pub provider: String,
    pub account_id: String,
    pub event_kind: String,
    pub entity_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_handle: Option<String>,
    pub timestamp: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    pub requires_attention: bool,
    #[serde(default)]
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RespondQueueListResponse {
    pub items: Vec<RespondQueueItem>,
    pub count: usize,
}
