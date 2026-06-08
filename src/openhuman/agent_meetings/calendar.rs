//! Calendar-triggered meeting identification.
//!
//! Filters heartbeat `PendingEvent`s to find events with a Google Meet URL
//! and extracts the data needed to create a `MeetingSession`.

use chrono::{DateTime, Utc};

use crate::openhuman::subconscious::heartbeat::planner::PendingEvent;

/// Hosts recognized as conferencing platforms.
const ALLOWED_HOSTS: &[&str] = &["meet.google.com", "zoom.us", "teams.microsoft.com"];

/// A calendar event identified as having a joinable meeting link.
#[derive(Debug, Clone)]
pub struct CalendarMeeting {
    pub title: String,
    pub meet_url: String,
    pub calendar_event_id: String,
    pub anchor_at: DateTime<Utc>,
}

/// Filter a list of planner events to those with a recognized meeting URL.
pub fn identify_meet_meetings(events: &[PendingEvent]) -> Vec<CalendarMeeting> {
    events
        .iter()
        .filter_map(|e| {
            let url = e.meet_url.as_deref()?;
            if !is_allowed_host(url) {
                return None;
            }
            Some(CalendarMeeting {
                title: e.title.clone(),
                meet_url: url.to_string(),
                calendar_event_id: e.source_event_id.clone(),
                anchor_at: e.anchor_at,
            })
        })
        .collect()
}

fn is_allowed_host(url: &str) -> bool {
    let host = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .and_then(|s| s.split('/').next())
        .unwrap_or("");
    ALLOWED_HOSTS
        .iter()
        .any(|allowed| host == *allowed || host.ends_with(&format!(".{allowed}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::subconscious::heartbeat::planner::{HeartbeatCategory, PendingEvent};

    fn make_event(meet_url: Option<&str>) -> PendingEvent {
        PendingEvent {
            category: HeartbeatCategory::Meetings,
            source: "calendar:googlecalendar".into(),
            source_event_id: "evt-1".into(),
            fingerprint: "fp".into(),
            overlap_key: "ok".into(),
            title: "Standup".into(),
            body: String::new(),
            deep_link: Some("https://calendar.google.com/event?id=x".into()),
            meet_url: meet_url.map(Into::into),
            anchor_at: Utc::now(),
        }
    }

    #[test]
    fn identifies_google_meet() {
        let events = vec![make_event(Some("https://meet.google.com/abc-defg-hij"))];
        let meetings = identify_meet_meetings(&events);
        assert_eq!(meetings.len(), 1);
        assert_eq!(meetings[0].meet_url, "https://meet.google.com/abc-defg-hij");
        assert_eq!(meetings[0].title, "Standup");
    }

    #[test]
    fn skips_events_without_meet_url() {
        let events = vec![make_event(None)];
        assert!(identify_meet_meetings(&events).is_empty());
    }

    #[test]
    fn skips_unrecognized_hosts() {
        let events = vec![make_event(Some("https://unknown-platform.com/meeting"))];
        assert!(identify_meet_meetings(&events).is_empty());
    }

    #[test]
    fn recognizes_zoom_and_teams() {
        let events = vec![
            make_event(Some("https://zoom.us/j/123456")),
            make_event(Some("https://teams.microsoft.com/l/meet/abc")),
        ];
        let meetings = identify_meet_meetings(&events);
        assert_eq!(meetings.len(), 2);
    }
}
