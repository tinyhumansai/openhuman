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
    Meetings,
    Reminders,
    Important,
}

/// Wire payload emitted on the `core_notification` socket event. Short,
/// user-facing fields only — downstream UI shapes title/body/category into
/// its own notification item structure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CoreNotificationEvent {
    /// Unique id for this notification publish (e.g. `"cron:<job_id>:<ts>"`).
    /// Because the timestamp is embedded, each publish produces a distinct id —
    /// every cron run, webhook failure, or subagent event gets its own entry in
    /// the notification center rather than replacing a previous one.
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
    /// Optional action buttons displayed alongside the notification.
    /// Backward-compatible: old events without this field deserialize to `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<CoreNotificationAction>>,
    /// Opaque handle for the workspace this notification belongs to, when it
    /// belongs to one (#5966).
    ///
    /// The publish-time gate in `bus::should_announce` already refuses to
    /// broadcast a notification from a workspace the user has switched away
    /// from, but that decision and the broadcast are not one atomic step:
    /// resolving the active workspace and sending are separate, so a switch
    /// in between can still let one through. Carrying the identity turns a
    /// boolean taken at an instant into something the receiver can re-check
    /// whenever it renders, which is what actually closes the window.
    ///
    /// A *handle*, never `workspace_dir` itself — the path is under the
    /// user's home directory and this payload reaches every connected client.
    /// See [`workspace_handle`](crate::openhuman::config::workspace_handle).
    ///
    /// `None` means the notification is not workspace-bound (cron, webhook,
    /// sub-agent, rejected API key) and applies wherever it lands. Also
    /// `None` for rows persisted before this field existed, which is why it
    /// is `default` — a receiver must treat absence as "not bound", not as a
    /// mismatch, or upgrading would silently hide every stored notification.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Workspace revision at the moment the announcement gate passed, set
    /// only when [`Self::workspace`] is (#5966).
    ///
    /// Without it a receiver cannot tell the two ways a handle mismatch
    /// happens apart. `workspace_changed` and `core_notification` are
    /// broadcast by separate tasks, so a notification for the workspace the
    /// user just switched *to* can arrive before the switch that announces
    /// it — and a strict handle check would drop a valid alert the core had
    /// already verified. Comparing revisions separates that case ("this
    /// receiver is behind, catch up and accept") from the one the check
    /// exists for ("this is from a workspace already switched away from").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_revision: Option<u64>,
}

/// A single action button attached to a notification.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CoreNotificationAction {
    /// Machine-readable identifier for this action (e.g. `"approve"`, `"dismiss"`).
    pub action_id: String,
    /// Human-readable button label.
    pub label: String,
    /// Opaque payload forwarded back when the user clicks the button.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// Short subject / title text.
    pub title: String,
    /// Body / preview text.
    pub body: String,
    /// Full raw event payload from the recipe for downstream use.
    pub raw_payload: serde_json::Value,
    /// 0.0–1.0 importance score produced by the triage pipeline (optional).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub importance_score: Option<f32>,
    /// Triage action string: `"drop"` / `"acknowledge"` / `"react"` / `"escalate"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triage_action: Option<String>,
    /// One-sentence justification from the classifier.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triage_reason: Option<String>,
    /// Lifecycle status.
    pub status: NotificationStatus,
    /// Wall-clock time the notification arrived.
    pub received_at: DateTime<Utc>,
    /// Wall-clock time triage completed.
    #[serde(skip_serializing_if = "Option::is_none")]
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

/// Aggregate statistics for the notification intelligence pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationStats {
    pub total: i64,
    pub unread: i64,
    pub unscored: i64,
    pub by_provider: std::collections::HashMap<String, i64>,
    pub by_action: std::collections::HashMap<String, i64>,
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

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
