//! Heartbeat planner — evaluates upcoming events and dispatches proactive
//! notifications.
//!
//! # Module layout
//!
//! | File | Responsibility |
//! |------|----------------|
//! | `types.rs` | Shared data types (`HeartbeatCategory`, `PendingEvent`, …) |
//! | `collectors.rs` | Source-specific collectors (cron, calendar, notifications) |
//! | `plan.rs` | Delivery-window logic (`plan_delivery_for_event`) |
//! | `persistence.rs` | Durable notification persistence (`persist_heartbeat_alert`) |
//! | `utils.rs` | Pure helpers (`sanitize_preview`, `stable_key`) |
//! | `store.rs` | Dedupe store (`mark_sent`, `prune_old`) |

mod collectors;
mod persistence;
mod plan;
mod store;
mod types;
mod utils;

pub use types::PlannerRunSummary;

use std::collections::HashSet;

use chrono::{DateTime, Duration, Utc};

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::notifications::bus::publish_core_notification;
use crate::openhuman::notifications::types::CoreNotificationEvent;

use collectors::{
    collect_calendar_meetings, collect_cron_reminders, collect_relevant_notifications,
};
use persistence::persist_heartbeat_alert;
use plan::plan_delivery_for_event;
use utils::stable_key;

/// Evaluate all configured notification categories and dispatch any events that
/// fall within their delivery windows and have not already been sent.
pub async fn evaluate_and_dispatch(config: &Config, now: DateTime<Utc>) -> PlannerRunSummary {
    let mut summary = PlannerRunSummary::empty();

    if !(config.heartbeat.notify_meetings
        || config.heartbeat.notify_reminders
        || config.heartbeat.notify_relevant_events)
    {
        tracing::debug!("[heartbeat:planner] all categories disabled; skipping tick");
        return summary;
    }

    let mut events = Vec::new();

    if config.heartbeat.notify_reminders {
        events.extend(collect_cron_reminders(config, now));
    }

    if config.heartbeat.notify_meetings {
        events.extend(collect_calendar_meetings(config, now).await);
    }

    if config.heartbeat.notify_relevant_events {
        events.extend(collect_relevant_notifications(config, now));
    }

    summary.source_events = events.len();

    let mut seen_keys: HashSet<String> = HashSet::new();

    for event in events {
        let Some(plan) = plan_delivery_for_event(&event, config, now) else {
            continue;
        };

        // Use `overlap_key` (content-based: category + title + time-bucket) so
        // that identical underlying events surfaced by multiple sources
        // (e.g. the same meeting visible in both cron reminders and a calendar
        // connection) map to the same dedupe key and only one notification is
        // delivered.
        let dedupe_key = stable_key(&format!(
            "{}|{}|{}",
            event.category.as_str(),
            event.overlap_key,
            plan.stage
        ));

        // Overlapping sources in the same tick should still dedupe before hitting disk.
        if !seen_keys.insert(dedupe_key.clone()) {
            summary.deliveries_skipped_dedup += 1;
            continue;
        }

        summary.deliveries_attempted += 1;

        let id = format!(
            "heartbeat:{}:{}:{}",
            event.category.as_str(),
            plan.stage,
            &dedupe_key[..12]
        );

        // Persist the durable notification record BEFORE marking dedupe, so a
        // failed write doesn't permanently suppress future retries.
        if let Err(error) = persist_heartbeat_alert(config, &event, &plan, now) {
            tracing::warn!(
                dedupe_key = %dedupe_key,
                source = %event.source,
                source_event_id = %event.source_event_id,
                category = event.category.as_str(),
                stage = plan.stage,
                error = %error,
                "[heartbeat:planner] failed to persist heartbeat alert; skipping delivery"
            );
            continue;
        }

        let inserted = match store::mark_sent(
            config,
            &store::SentMarker {
                dedupe_key: &dedupe_key,
                event_fingerprint: &event.fingerprint,
                source: &event.source,
                category: event.category.as_str(),
                stage: plan.stage,
                sent_at: now,
            },
        ) {
            Ok(v) => v,
            Err(error) => {
                tracing::warn!(
                    dedupe_key = %dedupe_key,
                    source = %event.source,
                    source_event_id = %event.source_event_id,
                    category = event.category.as_str(),
                    error = %error,
                    "[heartbeat:planner] failed to persist dedupe marker"
                );
                continue;
            }
        };

        if !inserted {
            summary.deliveries_skipped_dedup += 1;
            continue;
        }

        publish_core_notification(CoreNotificationEvent {
            id,
            category: event.category.notification_category(),
            title: plan.title,
            body: plan.body,
            deep_link: event.deep_link.clone(),
            timestamp_ms: now.timestamp_millis().max(0) as u64,
        });

        if config.heartbeat.external_delivery_enabled && plan.allow_external {
            publish_global(DomainEvent::ProactiveMessageRequested {
                source: format!("heartbeat:{}", event.category.as_str()),
                message: plan.proactive_message,
                job_name: Some(format!("heartbeat-{}", event.category.as_str())),
            });
        }

        summary.deliveries_sent += 1;

        tracing::debug!(
            dedupe_key = %dedupe_key,
            source = %event.source,
            source_event_id = %event.source_event_id,
            category = event.category.as_str(),
            stage = plan.stage,
            "[heartbeat:planner] delivery sent"
        );
    }

    if let Err(error) = store::prune_old(config, now - Duration::days(14)) {
        tracing::warn!(error = %error, "[heartbeat:planner] prune_old failed");
    }

    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::cron::{self, Schedule};
    use crate::openhuman::notifications::subscribe_core_notifications;
    use chrono::TimeZone;
    use serde_json::json;
    use tempfile::TempDir;

    use collectors::extract_calendar_events;
    use plan::plan_delivery_for_event;
    use types::{HeartbeatCategory, PendingEvent};
    use utils::{compute_overlap_key, sanitize_preview};

    #[test]
    fn extract_calendar_events_reads_nested_payload() {
        let now = Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        let payload = json!({
            "items": [
                {
                    "id": "evt-1",
                    "summary": "Team sync",
                    "start": { "dateTime": "2026-05-08T10:20:00Z" },
                    "htmlLink": "https://calendar.google.com/event?evt=1"
                }
            ]
        });

        let events = extract_calendar_events(
            &payload,
            "googlecalendar",
            "conn-1",
            now,
            now + Duration::minutes(60),
        );

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].category, HeartbeatCategory::Meetings);
        assert_eq!(events[0].source_event_id, "evt-1");
        assert_eq!(events[0].title, "Team sync");
        assert_eq!(
            events[0].deep_link.as_deref(),
            Some("https://calendar.google.com/event?evt=1")
        );
    }

    /// Regression for issue #1714 — an event stored against a
    /// non-UTC zone (here `+05:30` IST) must still classify into the
    /// window when its UTC-normalised start falls inside it. The
    /// extractor relies on RFC3339 offset parsing rather than naive
    /// string comparison; this test pins that behaviour so a future
    /// regression to "compare local-date strings" gets caught.
    #[test]
    fn calendar_events_with_non_utc_timezone_offset_are_kept() {
        // Window covers 06:00 → 09:00 UTC on 2026-05-14.
        let now = Utc.with_ymd_and_hms(2026, 5, 14, 6, 0, 0).unwrap();
        let payload = json!({
            "items": [
                {
                    "id": "evt-ist",
                    "summary": "Daily stand-up",
                    "start": { "dateTime": "2026-05-14T12:00:00+05:30" },
                    "htmlLink": "https://calendar.google.com/event?evt=ist"
                }
            ]
        });

        let events = extract_calendar_events(
            &payload,
            "googlecalendar",
            "conn-ist",
            now,
            now + Duration::hours(3),
        );

        // 12:00 IST == 06:30 UTC — inside the window.
        assert_eq!(events.len(), 1, "IST-offset event must be kept: {events:?}");
        assert_eq!(events[0].source_event_id, "evt-ist");
        assert_eq!(events[0].title, "Daily stand-up");
    }

    #[test]
    fn all_day_calendar_events_are_skipped() {
        let now = Utc.with_ymd_and_hms(2026, 5, 8, 0, 0, 0).unwrap();
        let payload = json!({
            "items": [
                {
                    "id": "all-day-1",
                    "summary": "Birthday",
                    "start": { "date": "2026-05-08" }
                }
            ]
        });

        let events = extract_calendar_events(
            &payload,
            "googlecalendar",
            "conn-1",
            now,
            now + Duration::hours(24),
        );

        assert_eq!(
            events.len(),
            0,
            "all-day events should not be promoted to meetings"
        );
    }

    #[test]
    fn reminder_stage_prioritizes_due_window() {
        let mut config = Config::default();
        config.heartbeat.reminder_lookahead_minutes = 15;
        let now = Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        let event = PendingEvent {
            category: HeartbeatCategory::Reminders,
            source: "cron".to_string(),
            source_event_id: "job-1".to_string(),
            fingerprint: "fp-1".to_string(),
            overlap_key: compute_overlap_key(HeartbeatCategory::Reminders, "Pay rent", now),
            title: "Pay rent".to_string(),
            body: String::new(),
            deep_link: None,
            anchor_at: now,
        };

        let plan = plan_delivery_for_event(&event, &config, now).expect("plan");
        assert_eq!(plan.stage, "due");
        assert!(plan.allow_external);
    }

    #[test]
    fn meeting_stage_uses_heads_up_for_longer_lead() {
        let mut config = Config::default();
        config.heartbeat.meeting_lookahead_minutes = 120;
        let now = Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();
        let event = PendingEvent {
            category: HeartbeatCategory::Meetings,
            source: "calendar:googlecalendar".to_string(),
            source_event_id: "evt-1".to_string(),
            fingerprint: "fp-1".to_string(),
            overlap_key: compute_overlap_key(
                HeartbeatCategory::Meetings,
                "Planning",
                now + Duration::minutes(45),
            ),
            title: "Planning".to_string(),
            body: String::new(),
            deep_link: None,
            anchor_at: now + Duration::minutes(45),
        };

        let plan = plan_delivery_for_event(&event, &config, now).expect("plan");
        assert_eq!(plan.stage, "heads_up");
        assert!(!plan.allow_external);
    }

    #[test]
    fn sanitize_preview_trims_and_normalizes_whitespace() {
        let out = sanitize_preview("  hello   world  ", 30);
        assert_eq!(out, "hello world");

        let out = sanitize_preview("a very long sentence with many words", 10);
        assert!(out.ends_with('…'));
        assert!(out.chars().count() <= 10);
    }

    fn test_config(tmp: &TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().to_path_buf(),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn evaluate_and_dispatch_dedupes_across_ticks() {
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.heartbeat.notify_meetings = false;
        config.heartbeat.notify_relevant_events = false;
        config.heartbeat.notify_reminders = true;
        config.heartbeat.reminder_lookahead_minutes = 30;

        let now = Utc::now();
        let run_at = now + Duration::minutes(5);
        let schedule = Schedule::At { at: run_at };
        let _job = cron::add_shell_job(&config, Some("remind_me".to_string()), schedule, "echo hi")
            .expect("create cron reminder");

        let mut rx = subscribe_core_notifications();
        while rx.try_recv().is_ok() {}

        let first = evaluate_and_dispatch(&config, now).await;
        assert_eq!(first.deliveries_sent, 1);

        let second = evaluate_and_dispatch(&config, now).await;
        assert_eq!(second.deliveries_sent, 0);
        assert!(second.deliveries_skipped_dedup >= 1);
    }

    #[tokio::test]
    async fn heartbeat_provider_notifications_are_not_re_escalated() {
        use crate::openhuman::notifications::store as notifications_store;
        use crate::openhuman::notifications::types::{IntegrationNotification, NotificationStatus};

        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.heartbeat.notify_meetings = false;
        config.heartbeat.notify_reminders = false;
        config.heartbeat.notify_relevant_events = true;

        let now = Utc::now();

        // Simulate a previously-persisted heartbeat notification (triage_action="react",
        // status=Unread, importance_score=0.9) — exactly what persist_heartbeat_alert writes.
        let hb_notification = IntegrationNotification {
            id: "heartbeat:meetings:final_call:abc123def456".to_string(),
            provider: "heartbeat".to_string(),
            account_id: None,
            title: "Upcoming meeting: Team sync".to_string(),
            body: "Starts in about 5 minutes.".to_string(),
            raw_payload: serde_json::json!({"category": "meetings", "stage": "final_call"}),
            importance_score: Some(0.9),
            triage_action: Some("react".to_string()),
            triage_reason: Some("heartbeat proactive event".to_string()),
            status: NotificationStatus::Unread,
            received_at: now,
            scored_at: Some(now),
        };
        notifications_store::insert_if_not_recent(&config, &hb_notification).unwrap();

        // Planner must NOT re-escalate notifications it generated itself.
        let summary = evaluate_and_dispatch(&config, now).await;
        assert_eq!(
            summary.deliveries_sent, 0,
            "heartbeat provider notifications must not be re-escalated as Important events"
        );
    }

    #[test]
    fn overlap_key_same_for_cross_source_same_event() {
        // Two different sources that surface the same meeting at the same time
        // (within the 15-minute bucket) must produce the same overlap_key so
        // only one notification is dispatched.
        let anchor = Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();

        let key_from_calendar =
            compute_overlap_key(HeartbeatCategory::Meetings, "Team Standup", anchor);
        // A cron job with the same title and an anchor 2 minutes later (same
        // 15-minute bucket) — different source, same underlying event.
        let key_from_cron = compute_overlap_key(
            HeartbeatCategory::Meetings,
            "Team Standup",
            anchor + Duration::minutes(2),
        );

        assert_eq!(
            key_from_calendar, key_from_cron,
            "cross-source events in the same 15-min bucket must share an overlap_key"
        );
    }

    #[test]
    fn overlap_key_differs_for_different_titles_or_times() {
        let anchor = Utc.with_ymd_and_hms(2026, 5, 8, 10, 0, 0).unwrap();

        // Different title → different key.
        let key_a = compute_overlap_key(HeartbeatCategory::Meetings, "Team Standup", anchor);
        let key_b = compute_overlap_key(HeartbeatCategory::Meetings, "1:1 With Manager", anchor);
        assert_ne!(
            key_a, key_b,
            "different titles must produce different overlap keys"
        );

        // Same title but more than one bucket apart (>= 15 min) → different key.
        let key_c = compute_overlap_key(
            HeartbeatCategory::Meetings,
            "Team Standup",
            anchor + Duration::minutes(20),
        );
        assert_ne!(
            key_a, key_c,
            "events in different time buckets must produce different overlap keys"
        );

        // Different category → different key even with same title and time.
        let key_d = compute_overlap_key(HeartbeatCategory::Reminders, "Team Standup", anchor);
        assert_ne!(
            key_a, key_d,
            "different categories must produce different overlap keys"
        );
    }
}
