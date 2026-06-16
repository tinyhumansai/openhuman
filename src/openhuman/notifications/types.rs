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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn core_notification_backward_compat_no_actions() {
        let json = json!({
            "id": "test-1",
            "category": "system",
            "title": "Hello",
            "body": "World",
            "timestamp_ms": 123456
        });
        let event: CoreNotificationEvent = serde_json::from_value(json).unwrap();
        assert!(event.actions.is_none());
        assert!(event.deep_link.is_none());
    }

    #[test]
    fn core_notification_with_actions() {
        let json = json!({
            "id": "test-2",
            "category": "meetings",
            "title": "Join call?",
            "body": "Standup in 5 min",
            "timestamp_ms": 999,
            "actions": [
                {"actionId": "yes", "label": "Yes"},
                {"actionId": "no", "label": "No", "payload": {"meeting_id": "m1"}}
            ]
        });
        let event: CoreNotificationEvent = serde_json::from_value(json).unwrap();
        let actions = event.actions.unwrap();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_id, "yes");
        assert!(actions[0].payload.is_none());
        assert_eq!(actions[1].action_id, "no");
        assert!(actions[1].payload.is_some());
    }

    #[test]
    fn core_notification_serialize_skips_empty_actions() {
        let event = CoreNotificationEvent {
            id: "x".into(),
            category: CoreNotificationCategory::System,
            title: "t".into(),
            body: "b".into(),
            deep_link: None,
            timestamp_ms: 1,
            actions: None,
        };
        let s = serde_json::to_string(&event).unwrap();
        assert!(!s.contains("actions"));
    }
}
