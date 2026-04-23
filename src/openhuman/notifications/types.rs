use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Core-bridge types (DomainEvent → socket.io → frontend notification center)
// ---------------------------------------------------------------------------

/// Category used by the frontend notification center to apply per-category
/// preferences. Matches `NotificationCategory` in
/// `app/src/store/notificationSlice.ts` — keep the two in sync.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CoreNotificationCategory {
    Messages,
    Agents,
    Skills,
    System,
}

/// Wire payload emitted on the `core_notification` socket event. Short,
/// user-facing fields only — downstream UI shapes title/body/category into
/// its own notification item structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreNotificationEvent {
    /// Stable id used for de-duplication in the center (e.g.
    /// `"cron:<job_id>:<ts>"`). The frontend keys by this id so repeated
    /// publishes for the same logical event don't pile up.
    pub id: String,
    pub category: CoreNotificationCategory,
    pub title: String,
    pub body: String,
    /// Optional in-app deep link the user is sent to when they click the
    /// notification (mirrors the `deepLink` field on the frontend item).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deep_link: Option<String>,
    /// Wall-clock milliseconds since the unix epoch at publish time.
    pub timestamp_ms: u64,
}

// ---------------------------------------------------------------------------
// Integration notification types (webview recipe events → triage pipeline)
// ---------------------------------------------------------------------------

/// Lifecycle state for an ingested notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "snake_case")]
pub enum NotificationStatus {
    #[default]
    Unread,
    Read,
    Acted,
    Dismissed,
}

impl NotificationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unread => "unread",
            Self::Read => "read",
            Self::Acted => "acted",
            Self::Dismissed => "dismissed",
        }
    }
}

/// A single notification captured from an embedded webview integration.
///
/// Notifications are written on ingest and enriched in-place once the
/// triage pipeline produces its score/action. The `importance_score`,
/// `triage_action`, and `triage_reason` fields are `None` until the
/// background triage task completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IntegrationNotification {
    pub id: String,
    /// Provider slug: `"gmail"`, `"slack"`, `"whatsapp"`, etc.
    pub provider: String,
    /// Webview account id if the notification came from an embedded account.
    pub account_id: Option<String>,
    /// Short subject / title text.
    pub title: String,
    /// Body / preview text.
    pub body: String,
    /// Full raw event payload from the recipe for downstream use.
    pub raw_payload: serde_json::Value,
    /// 0.0–1.0 importance score produced by the triage pipeline (optional).
    pub importance_score: Option<f32>,
    /// Triage action string: `"drop"` / `"acknowledge"` / `"react"` / `"escalate"`.
    pub triage_action: Option<String>,
    /// One-sentence justification from the classifier.
    pub triage_reason: Option<String>,
    /// Lifecycle status.
    pub status: NotificationStatus,
    /// Wall-clock time the notification arrived.
    pub received_at: DateTime<Utc>,
    /// Wall-clock time triage completed.
    pub scored_at: Option<DateTime<Utc>>,
}

/// Per-provider user preference controlling which notifications surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    pub provider: String,
    /// Whether notifications from this provider should be ingested at all.
    pub enabled: bool,
    /// Minimum importance score (0.0–1.0) to display; 0.0 = show all.
    pub importance_threshold: f32,
    /// When `true`, triage-escalated notifications are also auto-forwarded to
    /// the orchestrator agent.
    pub route_to_orchestrator: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            provider: String::new(),
            enabled: true,
            importance_threshold: 0.0,
            route_to_orchestrator: true,
        }
    }
}

/// Payload for the `notification_ingest` RPC endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationIngestRequest {
    /// Provider slug: `"gmail"`, `"slack"`, etc.
    pub provider: String,
    /// Webview account id (optional).
    pub account_id: Option<String>,
    /// Human-readable notification title.
    pub title: String,
    /// Notification body / preview.
    pub body: String,
    /// Full raw payload from the source.
    pub raw_payload: serde_json::Value,
}

/// Payload for `notification_settings_set`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettingsUpsertRequest {
    pub provider: String,
    pub enabled: bool,
    pub importance_threshold: f32,
    pub route_to_orchestrator: bool,
}
