use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::openhuman::notifications::types::CoreNotificationCategory;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HeartbeatCategory {
    Meetings,
    Reminders,
    Important,
}

impl HeartbeatCategory {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Meetings => "meetings",
            Self::Reminders => "reminders",
            Self::Important => "important",
        }
    }

    pub(crate) fn notification_category(&self) -> CoreNotificationCategory {
        match self {
            Self::Meetings => CoreNotificationCategory::Meetings,
            Self::Reminders => CoreNotificationCategory::Reminders,
            Self::Important => CoreNotificationCategory::Important,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PendingEvent {
    pub category: HeartbeatCategory,
    pub source: String,
    pub source_event_id: String,
    /// Source-specific fingerprint — unique within a single source.
    pub fingerprint: String,
    /// Content-based overlap key — identical events from different sources
    /// (e.g. the same meeting appearing in both a cron job and a calendar
    /// connection) hash to the same value and are deduplicated across sources.
    /// Derived from `category + normalized_title + time_bucket`.
    pub overlap_key: String,
    pub title: String,
    pub body: String,
    pub deep_link: Option<String>,
    pub anchor_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub(crate) struct PlannedDelivery {
    pub stage: &'static str,
    pub title: String,
    pub body: String,
    pub proactive_message: String,
    pub allow_external: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct PlannerRunSummary {
    pub source_events: usize,
    pub deliveries_attempted: usize,
    pub deliveries_sent: usize,
    pub deliveries_skipped_dedup: usize,
}

impl PlannerRunSummary {
    pub(crate) fn empty() -> Self {
        Self {
            source_events: 0,
            deliveries_attempted: 0,
            deliveries_sent: 0,
            deliveries_skipped_dedup: 0,
        }
    }
}
