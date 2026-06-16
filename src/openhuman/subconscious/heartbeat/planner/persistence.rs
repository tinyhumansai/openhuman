use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::notifications::store as notifications_store;
use crate::openhuman::notifications::types::{IntegrationNotification, NotificationStatus};

use super::types::{HeartbeatCategory, PendingEvent, PlannedDelivery};
use super::utils::sanitize_preview;

/// Durably persist a heartbeat alert into the notifications store.
///
/// Returns an error if the store write fails. The caller should refrain from
/// marking the dedupe key until this returns `Ok`, so that a failed write does
/// not permanently suppress future retries.
pub(crate) fn persist_heartbeat_alert(
    config: &Config,
    event: &PendingEvent,
    plan: &PlannedDelivery,
    now: DateTime<Utc>,
) -> anyhow::Result<()> {
    let notification = IntegrationNotification {
        id: format!(
            "heartbeat:{}:{}:{}",
            event.category.as_str(),
            plan.stage,
            &event.fingerprint[..12]
        ),
        provider: "heartbeat".to_string(),
        account_id: Some(event.source_event_id.clone()),
        title: sanitize_preview(&plan.title, 100),
        body: sanitize_preview(&plan.body, 180),
        raw_payload: serde_json::json!({
            "source": event.source,
            "category": event.category.as_str(),
            "stage": plan.stage,
            "anchor_at": event.anchor_at.to_rfc3339(),
            "deep_link": event.deep_link.clone(),
        }),
        importance_score: Some(match event.category {
            HeartbeatCategory::Meetings => 0.8,
            HeartbeatCategory::Reminders => 0.7,
            HeartbeatCategory::Important => 0.9,
        }),
        triage_action: Some("react".to_string()),
        triage_reason: Some("heartbeat proactive event".to_string()),
        status: NotificationStatus::Unread,
        received_at: now,
        scored_at: Some(now),
    };

    notifications_store::insert_if_not_recent(config, &notification).map(|_| ())
}
