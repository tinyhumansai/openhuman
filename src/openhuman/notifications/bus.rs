//! Broadcast bus + DomainEvent subscriber for core notifications.
//!
//! Mirrors the pattern used by [`overlay::bus`](crate::openhuman::overlay::bus)
//! — a single `tokio::sync::broadcast` channel wrapped in a `Lazy` static,
//! plus a [`EventHandler`] implementation that translates relevant
//! [`DomainEvent`] variants into [`CoreNotificationEvent`] payloads.
//!
//! The Socket.IO bridge in `core::socketio::spawn_web_channel_bridge`
//! subscribes to this bus and forwards every event to all connected clients
//! as `core_notification` / `core:notification` Socket.IO messages.

use once_cell::sync::Lazy;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;

use super::types::{CoreNotificationCategory, CoreNotificationEvent};

const LOG_PREFIX: &str = "[core-notify]";

static NOTIFICATION_BUS: Lazy<broadcast::Sender<CoreNotificationEvent>> = Lazy::new(|| {
    let (tx, _rx) = broadcast::channel(128);
    tx
});

/// Subscribe to core notifications — consumed by the Socket.IO bridge at
/// startup. Additional in-process consumers (e.g. integration tests) can
/// subscribe too.
pub fn subscribe_core_notifications() -> broadcast::Receiver<CoreNotificationEvent> {
    NOTIFICATION_BUS.subscribe()
}

/// Publish a core notification. Fire-and-forget: if nobody is currently
/// subscribed the event is dropped. Returns the number of active
/// subscribers that received the event for diagnostics.
pub fn publish_core_notification(event: CoreNotificationEvent) -> usize {
    log::debug!(
        "{LOG_PREFIX} publish id={} category={:?} title_chars={}",
        event.id,
        event.category,
        event.title.len(),
    );
    NOTIFICATION_BUS.send(event).unwrap_or(0)
}

/// Subscribes to selected DomainEvent variants and translates each into a
/// [`CoreNotificationEvent`]. Pure translation — no I/O, no locks.
#[derive(Default)]
pub struct NotificationBridgeSubscriber;

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Pure translation function — kept free so unit tests can drive it
/// without spinning up tokio or the broadcast channel.
pub fn event_to_notification(event: &DomainEvent) -> Option<CoreNotificationEvent> {
    let ts = now_ms();
    match event {
        DomainEvent::CronJobCompleted {
            job_id, success, ..
        } => Some(CoreNotificationEvent {
            id: format!("cron:{}:{}", job_id, ts),
            category: CoreNotificationCategory::Agents,
            title: if *success {
                "Cron job completed".into()
            } else {
                "Cron job failed".into()
            },
            body: if *success {
                format!("Job {job_id} finished successfully.")
            } else {
                format!("Job {job_id} did not complete — check your cron schedule.")
            },
            deep_link: Some("/settings/cron-jobs".into()),
            timestamp_ms: ts,
        }),
        DomainEvent::WebhookProcessed {
            skill_id,
            status_code,
            elapsed_ms,
            error,
            ..
        } => {
            // Only surface failures — successful webhooks are noisy.
            if error.is_none() && *status_code < 400 {
                return None;
            }
            Some(CoreNotificationEvent {
                id: format!("webhook:{}:{}", skill_id, ts),
                category: CoreNotificationCategory::System,
                title: "Webhook error".into(),
                body: match error {
                    Some(err) => {
                        format!("{skill_id} webhook failed after {elapsed_ms}ms: {err}")
                    }
                    None => format!(
                        "{skill_id} webhook returned HTTP {status_code} after {elapsed_ms}ms"
                    ),
                },
                deep_link: Some("/webhooks".into()),
                timestamp_ms: ts,
            })
        }
        DomainEvent::SubagentCompleted {
            parent_session,
            task_id,
            agent_id,
            output_chars,
            ..
        } => Some(CoreNotificationEvent {
            id: format!("subagent:{}:{}:{}", parent_session, task_id, ts),
            category: CoreNotificationCategory::Agents,
            title: "Sub-agent finished".into(),
            body: format!("{agent_id} produced {output_chars} chars of output."),
            deep_link: Some("/chat".into()),
            timestamp_ms: ts,
        }),
        DomainEvent::SubagentFailed {
            parent_session,
            task_id,
            agent_id,
            error,
        } => Some(CoreNotificationEvent {
            id: format!("subagent:{}:{}:{}", parent_session, task_id, ts),
            category: CoreNotificationCategory::Agents,
            title: "Sub-agent failed".into(),
            body: format!(
                "{agent_id} encountered an error: {}",
                error.chars().take(100).collect::<String>()
            ),
            deep_link: Some("/chat".into()),
            timestamp_ms: ts,
        }),
        DomainEvent::NotificationTriaged {
            id,
            provider,
            action,
            importance_score,
            latency_ms,
            routed,
        } if *routed && (action == "escalate" || action == "react") => {
            Some(CoreNotificationEvent {
                id: format!("notification-triaged:{}:{}:{}", id, action, latency_ms),
                category: CoreNotificationCategory::Agents,
                title: format!("High-priority {} notification", provider),
                body: if action == "escalate" {
                    format!(
                        "Action: escalate (score: {:.0}%). Routed to orchestrator.",
                        importance_score * 100.0
                    )
                } else {
                    format!(
                        "Action: react (score: {:.0}%). Routed for follow-up.",
                        importance_score * 100.0
                    )
                },
                deep_link: Some("/notifications".into()),
                timestamp_ms: ts,
            })
        }
        _ => None,
    }
}

#[async_trait]
impl EventHandler for NotificationBridgeSubscriber {
    fn name(&self) -> &str {
        "notifications::bridge"
    }

    // `domains()` returns None — we filter at the variant match instead of
    // the domain string, since we pull from three different domains and
    // the domain list is an optional short-circuit rather than a
    // correctness boundary.

    async fn handle(&self, event: &DomainEvent) {
        if let Some(notification) = event_to_notification(event) {
            publish_core_notification(notification);
        }
    }
}

/// Register the notification bridge subscriber on the global event bus.
/// Safe to call multiple times — each call produces a fresh subscription,
/// but the caller (`register_domain_subscribers`) is Once-guarded.
pub fn register_notification_bridge_subscriber() {
    use std::sync::Arc;
    if let Some(handle) =
        crate::core::event_bus::subscribe_global(Arc::new(NotificationBridgeSubscriber::default()))
    {
        // SAFETY: intentional leak; handle's Drop would cancel the subscriber.
        std::mem::forget(handle);
        log::info!("{LOG_PREFIX} notification bridge subscriber registered");
    } else {
        log::warn!(
            "{LOG_PREFIX} failed to register notification bridge — event bus not initialized"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_completed_produces_agents_notification() {
        let ev = DomainEvent::CronJobCompleted {
            job_id: "job-1".into(),
            success: true,
            output: "done".into(),
        };
        let n = event_to_notification(&ev).expect("should produce notification");
        assert_eq!(n.category, CoreNotificationCategory::Agents);
        assert_eq!(n.title, "Cron job completed");
        assert!(n.body.contains("job-1"));
    }

    #[test]
    fn cron_failed_uses_failure_title() {
        let ev = DomainEvent::CronJobCompleted {
            job_id: "job-1".into(),
            success: false,
            output: "error".into(),
        };
        let n = event_to_notification(&ev).unwrap();
        assert_eq!(n.title, "Cron job failed");
    }

    #[test]
    fn successful_webhook_is_silent() {
        let ev = DomainEvent::WebhookProcessed {
            tunnel_id: "t".into(),
            skill_id: "s".into(),
            method: "POST".into(),
            path: "/p".into(),
            correlation_id: "c".into(),
            status_code: 200,
            elapsed_ms: 5,
            error: None,
        };
        assert!(event_to_notification(&ev).is_none());
    }

    #[test]
    fn failed_webhook_produces_system_notification() {
        let ev = DomainEvent::WebhookProcessed {
            tunnel_id: "t".into(),
            skill_id: "skill-x".into(),
            method: "POST".into(),
            path: "/p".into(),
            correlation_id: "c".into(),
            status_code: 500,
            elapsed_ms: 12,
            error: Some("boom".into()),
        };
        let n = event_to_notification(&ev).unwrap();
        assert_eq!(n.category, CoreNotificationCategory::System);
        assert!(n.body.contains("skill-x"));
        assert!(n.body.contains("boom"));
    }

    #[test]
    fn subagent_completed_produces_agents_notification() {
        let ev = DomainEvent::SubagentCompleted {
            parent_session: "p".into(),
            task_id: "t".into(),
            agent_id: "researcher".into(),
            elapsed_ms: 100,
            output_chars: 500,
            iterations: 3,
        };
        let n = event_to_notification(&ev).unwrap();
        assert_eq!(n.category, CoreNotificationCategory::Agents);
        assert!(n.body.contains("researcher"));
        assert!(n.body.contains("500"));
    }

    #[test]
    fn subagent_failed_produces_agents_notification() {
        let ev = DomainEvent::SubagentFailed {
            parent_session: "p".into(),
            task_id: "t".into(),
            agent_id: "researcher".into(),
            error: "context window exceeded".into(),
        };
        let n = event_to_notification(&ev).unwrap();
        assert_eq!(n.category, CoreNotificationCategory::Agents);
        assert_eq!(n.title, "Sub-agent failed");
        assert!(n.body.contains("researcher"));
        assert!(n.body.contains("context window exceeded"));
    }

    #[test]
    fn unrelated_events_return_none() {
        let ev = DomainEvent::AgentTurnCompleted {
            session_id: "s".into(),
            text_chars: 1,
            iterations: 1,
        };
        assert!(event_to_notification(&ev).is_none());
    }

    #[test]
    fn notification_triaged_escalate_produces_agents_notification() {
        let ev = DomainEvent::NotificationTriaged {
            id: "n1".into(),
            provider: "slack".into(),
            action: "escalate".into(),
            importance_score: 0.9,
            latency_ms: 100,
            routed: true,
        };
        let n = event_to_notification(&ev).expect("should produce notification");
        assert_eq!(n.category, CoreNotificationCategory::Agents);
        assert!(n.body.contains("escalate"));
        assert!(n.deep_link.as_deref() == Some("/notifications"));
    }

    #[test]
    fn notification_triaged_react_uses_follow_up_copy() {
        let ev = DomainEvent::NotificationTriaged {
            id: "n2".into(),
            provider: "discord".into(),
            action: "react".into(),
            importance_score: 0.7,
            latency_ms: 120,
            routed: true,
        };
        let n = event_to_notification(&ev).expect("should produce notification");
        assert_eq!(n.category, CoreNotificationCategory::Agents);
        assert!(n.body.contains("Routed for follow-up"));
    }

    #[test]
    fn notification_triaged_drop_is_silent() {
        let ev = DomainEvent::NotificationTriaged {
            id: "n1".into(),
            provider: "gmail".into(),
            action: "drop".into(),
            importance_score: 0.1,
            latency_ms: 50,
            routed: false,
        };
        assert!(event_to_notification(&ev).is_none());
    }

    #[test]
    fn notification_triaged_unrouted_escalate_is_silent() {
        let ev = DomainEvent::NotificationTriaged {
            id: "n1".into(),
            provider: "slack".into(),
            action: "escalate".into(),
            importance_score: 0.9,
            latency_ms: 100,
            routed: false,
        };
        assert!(event_to_notification(&ev).is_none());
    }
}
