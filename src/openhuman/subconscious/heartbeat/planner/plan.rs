use chrono::{DateTime, Duration, Utc};

use crate::openhuman::config::Config;

use super::types::{HeartbeatCategory, PendingEvent, PlannedDelivery};

/// Choose the correct notification stage and message text for `event` given
/// the current time and user config. Returns `None` when the event is outside
/// all delivery windows and should be skipped.
pub(crate) fn plan_delivery_for_event(
    event: &PendingEvent,
    config: &Config,
    now: DateTime<Utc>,
) -> Option<PlannedDelivery> {
    let until = event.anchor_at.signed_duration_since(now);
    let until_minutes = until.num_minutes();

    match event.category {
        HeartbeatCategory::Meetings => {
            let lookahead = i64::from(config.heartbeat.meeting_lookahead_minutes.max(1));
            if until_minutes > 10 && until_minutes <= lookahead {
                let mins = until_minutes.max(1);
                return Some(PlannedDelivery {
                    stage: "heads_up",
                    title: format!("Meeting soon: {}", event.title),
                    body: format!("Starts in about {mins} minutes."),
                    proactive_message: format!(
                        "You have a meeting coming up in about {mins} minutes: {}.",
                        event.title
                    ),
                    allow_external: false,
                });
            }
            if until_minutes > 0 && until_minutes <= 10 {
                let mins = until_minutes.max(1);
                return Some(PlannedDelivery {
                    stage: "final_call",
                    title: format!("Upcoming meeting: {}", event.title),
                    body: format!("Starts in about {mins} minutes."),
                    proactive_message: format!(
                        "Your meeting starts in about {mins} minutes: {}.",
                        event.title
                    ),
                    allow_external: true,
                });
            }
            // Wider grace window: heartbeat runs every few minutes, so
            // tiny post-start windows can miss real meetings.
            if until_minutes <= 0 && until_minutes >= -10 {
                return Some(PlannedDelivery {
                    stage: "starting_now",
                    title: format!("Meeting starting now: {}", event.title),
                    body: "This meeting should be starting now.".to_string(),
                    proactive_message: format!("Your meeting is starting now: {}.", event.title),
                    allow_external: true,
                });
            }
            None
        }
        HeartbeatCategory::Reminders => {
            let lookahead = i64::from(config.heartbeat.reminder_lookahead_minutes.max(1));
            if until_minutes > 0 && until_minutes <= lookahead {
                let mins = until_minutes.max(1);
                return Some(PlannedDelivery {
                    stage: "soon",
                    title: format!("Reminder soon: {}", event.title),
                    body: format!("Scheduled in about {mins} minutes."),
                    proactive_message: format!(
                        "Reminder coming up in about {mins} minutes: {}.",
                        event.title
                    ),
                    allow_external: false,
                });
            }
            // Wider grace window for reminder due state to prevent misses
            // from tick alignment.
            if until_minutes <= 0 && until_minutes >= -10 {
                return Some(PlannedDelivery {
                    stage: "due",
                    title: format!("Reminder due: {}", event.title),
                    body: "A scheduled reminder is due now.".to_string(),
                    proactive_message: format!("Reminder due now: {}.", event.title),
                    allow_external: true,
                });
            }
            None
        }
        HeartbeatCategory::Important => {
            if now.signed_duration_since(event.anchor_at) <= Duration::minutes(10) {
                return Some(PlannedDelivery {
                    stage: "important_now",
                    title: event.title.clone(),
                    body: if event.body.is_empty() {
                        "A time-sensitive event needs your attention.".to_string()
                    } else {
                        event.body.clone()
                    },
                    proactive_message: if event.body.is_empty() {
                        "A time-sensitive event needs your attention.".to_string()
                    } else {
                        event.body.clone()
                    },
                    allow_external: true,
                });
            }
            None
        }
    }
}
