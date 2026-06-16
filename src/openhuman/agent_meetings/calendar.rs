//! Calendar-triggered meeting auto-join subscriber.
//!
//! Listens for [`DomainEvent::ComposioTriggerReceived`] events from the
//! `googlecalendar` toolkit and, when the payload contains a Google Meet
//! link, either auto-joins or notifies the user based on
//! `config.meet.auto_join_policy`.
//!
//! ## Trigger flow
//!
//! ```text
//! Google Calendar event created/updated
//!   └─► Composio fires webhook
//!         └─► backend verifies + emits `composio:trigger` over Socket.IO
//!               └─► core publishes `ComposioTriggerReceived`
//!                     └─► `MeetCalendarSubscriber` (this module)
//!                           ├─► policy = "always" → emit `bot:join`
//!                           ├─► policy = "ask"    → publish `MeetAutoJoinPrompt`
//!                           └─► policy = "never"  → drop
//! ```

use std::sync::OnceLock;

use async_trait::async_trait;

use crate::core::event_bus::{
    publish_global, subscribe_global, DomainEvent, EventHandler, SubscriptionHandle,
};
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::notifications::bus::publish_core_notification;
use crate::openhuman::notifications::types::{
    CoreNotificationAction, CoreNotificationCategory, CoreNotificationEvent,
};

use super::store;
use super::types::{AutoJoinSource, MeetingSession, MeetingSessionStatus};

static MEET_CALENDAR_HANDLE: OnceLock<SubscriptionHandle> = OnceLock::new();

/// Register the calendar-triggered meeting subscriber. Idempotent.
pub fn register_meet_calendar_subscriber() {
    if MEET_CALENDAR_HANDLE.get().is_some() {
        return;
    }
    match subscribe_global(std::sync::Arc::new(MeetCalendarSubscriber)) {
        Some(handle) => {
            let _ = MEET_CALENDAR_HANDLE.set(handle);
            tracing::debug!("[event_bus] meet calendar subscriber registered");
        }
        None => {
            tracing::warn!(
                "[event_bus] failed to register meet calendar subscriber — bus not initialized"
            );
        }
    }
}

/// Subscriber that reacts to Google Calendar Composio triggers.
struct MeetCalendarSubscriber;

#[async_trait]
impl EventHandler for MeetCalendarSubscriber {
    fn name(&self) -> &str {
        "agent_meetings::calendar"
    }

    fn domains(&self) -> Option<&[&str]> {
        // Listen on the composio domain since that's where
        // `ComposioTriggerReceived` events are published.
        Some(&["composio"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ComposioTriggerReceived {
            toolkit,
            trigger,
            payload,
            ..
        } = event
        else {
            return;
        };

        // Only care about Google Calendar triggers.
        if !toolkit.eq_ignore_ascii_case("googlecalendar") {
            return;
        }

        tracing::debug!(
            trigger = %trigger,
            "[meet:calendar] received googlecalendar trigger"
        );

        // Extract a Google Meet URL from the calendar event payload.
        // Composio sends different shapes depending on the trigger, but
        // the Meet link typically lives in one of these locations:
        //   - payload.hangoutLink (direct field on calendar event)
        //   - payload.conferenceData.entryPoints[].uri
        //   - deeply nested inside payload.data.* variants
        let meet_url = extract_meet_url(payload);
        let Some(meet_url) = meet_url else {
            tracing::debug!(
                trigger = %trigger,
                "[meet:calendar] no Google Meet URL found in payload, skipping"
            );
            return;
        };

        // Only act on meetings that are starting soon (within 10 minutes)
        // or already in progress. Skip events that are far in the future
        // or already ended.
        if !is_meeting_imminent(payload) {
            tracing::debug!(
                trigger = %trigger,
                "[meet:calendar] meeting is not imminent, skipping"
            );
            return;
        }

        let event_title = payload
            .get("summary")
            .or_else(|| payload.get("title"))
            .or_else(|| {
                payload
                    .get("data")
                    .and_then(|d| d.get("summary").or_else(|| d.get("title")))
            })
            .and_then(|v| v.as_str())
            .unwrap_or("Untitled meeting")
            .to_string();

        tracing::info!(
            trigger = %trigger,
            meet_url = %meet_url,
            title = %event_title,
            "[meet:calendar] detected imminent Google Meet meeting"
        );

        // Check the auto-join policy.
        let config = match config_rpc::load_config_with_timeout().await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "[meet:calendar] failed to load config, defaulting to ask"
                );
                // Publish prompt as fallback.
                publish_global(DomainEvent::MeetAutoJoinPrompt {
                    meet_url,
                    event_title,
                });
                return;
            }
        };

        match config.meet.auto_join_policy {
            crate::openhuman::config::schema::AutoJoinPolicy::Never => {
                tracing::debug!("[meet:calendar] auto_join_policy=never, dropping");
                return;
            }
            crate::openhuman::config::schema::AutoJoinPolicy::Always => {
                tracing::info!(
                    meet_url = %meet_url,
                    title = %event_title,
                    "[meet:calendar] auto_join_policy=always, joining automatically"
                );
                let correlation_id = uuid::Uuid::new_v4().to_string();
                tokio::spawn(auto_join_meeting(
                    meet_url,
                    event_title,
                    correlation_id,
                    true, // calendar auto-join bots are passive listeners by default
                ));
                return;
            }
            crate::openhuman::config::schema::AutoJoinPolicy::AskEachTime => {
                // Default: ask — create a Pending session and surface an
                // actionable notification (issue #3507). The buttons route
                // through `agent_meetings_notification_action`.
                tracing::info!(
                    meet_url = %meet_url,
                    title = %event_title,
                    "[meet:calendar] auto_join_policy=ask_each_time, prompting user"
                );

                // Dedupe: one prompt per meeting URL while a session is
                // still open (Composio can re-fire the trigger on event
                // updates).
                if let Ok(Some(existing)) = store::get_session_by_meet_url(&config, &meet_url) {
                    if existing.status != MeetingSessionStatus::Ended {
                        tracing::debug!(
                            meeting_id = %existing.id,
                            "[meet:calendar] open session already exists — skipping re-prompt"
                        );
                        return;
                    }
                }

                let meeting_id = uuid::Uuid::new_v4().to_string();
                let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
                let session = MeetingSession {
                    id: meeting_id.clone(),
                    meet_url: meet_url.clone(),
                    title: Some(event_title.clone()),
                    calendar_event_id: None,
                    status: MeetingSessionStatus::Pending,
                    source: AutoJoinSource::Calendar,
                    thread_id: None,
                    transcript_received: false,
                    summary_generated: false,
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                };
                if let Err(e) = store::create_session(&config, &session) {
                    tracing::warn!("[meet:calendar] session create failed (non-fatal): {e}");
                }

                let action_payload = serde_json::json!({
                    "meetingId": meeting_id,
                    "meetUrl": meet_url,
                    "title": event_title,
                });
                let action = |action_id: &str, label: &str| CoreNotificationAction {
                    action_id: action_id.to_string(),
                    label: label.to_string(),
                    payload: Some(action_payload.clone()),
                };
                publish_core_notification(CoreNotificationEvent {
                    id: format!("meet-auto-join:{meeting_id}"),
                    category: CoreNotificationCategory::Meetings,
                    title: format!("Meeting starting: {event_title}"),
                    body: "Add Tiny to this meeting?".to_string(),
                    deep_link: None,
                    timestamp_ms: now_ms,
                    actions: Some(vec![
                        action("join_listen", "Join (listen only)"),
                        action("join_active", "Join & reply"),
                        action("skip", "Not this one"),
                        action("always_join", "Always join"),
                    ]),
                });

                // Legacy prompt event kept for existing consumers.
                publish_global(DomainEvent::MeetAutoJoinPrompt {
                    meet_url,
                    event_title,
                });
            }
        }
    }
}

/// Maximum number of minutes before a meeting starts to consider it "imminent".
const IMMINENT_WINDOW_MINUTES: i64 = 10;

/// Check whether a calendar event is starting soon or already in progress.
///
/// Returns `true` when:
/// - The event's start time is within [`IMMINENT_WINDOW_MINUTES`] from now, or
/// - The event has already started but hasn't ended yet, or
/// - No start time can be parsed (fail-open to avoid silently dropping events).
fn is_meeting_imminent(payload: &serde_json::Value) -> bool {
    let now = chrono::Utc::now();

    // Try to find start/end times. Google Calendar API uses:
    //   start.dateTime (RFC3339) or start.date (all-day)
    //   end.dateTime or end.date
    // Composio may nest under `data`.
    let roots = [payload, payload.get("data").unwrap_or(payload)];

    for root in &roots {
        let start_str = root
            .get("start")
            .and_then(|s| s.get("dateTime").or_else(|| s.get("date_time")))
            .and_then(|v| v.as_str())
            .or_else(|| root.get("startTime").and_then(|v| v.as_str()))
            .or_else(|| root.get("start_time").and_then(|v| v.as_str()));

        let end_str = root
            .get("end")
            .and_then(|e| e.get("dateTime").or_else(|| e.get("date_time")))
            .and_then(|v| v.as_str())
            .or_else(|| root.get("endTime").and_then(|v| v.as_str()))
            .or_else(|| root.get("end_time").and_then(|v| v.as_str()));

        if let Some(start_str) = start_str {
            if let Ok(start) = chrono::DateTime::parse_from_rfc3339(start_str) {
                let start_utc = start.with_timezone(&chrono::Utc);
                let minutes_until_start = (start_utc - now).num_minutes();

                // Already ended?
                if let Some(end_str) = end_str {
                    if let Ok(end) = chrono::DateTime::parse_from_rfc3339(end_str) {
                        if end.with_timezone(&chrono::Utc) < now {
                            tracing::debug!(
                                start = %start_str,
                                end = %end_str,
                                "[meet:calendar] meeting already ended"
                            );
                            return false;
                        }
                    }
                }

                // Starting within the window or already started?
                let imminent = minutes_until_start <= IMMINENT_WINDOW_MINUTES;
                tracing::debug!(
                    start = %start_str,
                    minutes_until_start = minutes_until_start,
                    imminent = imminent,
                    "[meet:calendar] meeting start check"
                );
                return imminent;
            }
        }
    }

    // No parseable start time — fail-open so we don't silently drop.
    tracing::debug!("[meet:calendar] no start time found in payload, treating as imminent");
    true
}

/// Supported meeting URL host patterns. A string is considered a meeting
/// link when it contains any of these substrings.
const MEETING_HOST_PATTERNS: &[&str] = &[
    "meet.google.com",
    "zoom.us",
    "teams.microsoft.com",
    "webex.com",
];

fn is_meeting_url(s: &str) -> bool {
    MEETING_HOST_PATTERNS.iter().any(|pat| s.contains(pat))
}

/// Extract a meeting URL from a Composio Google Calendar trigger payload.
///
/// Supports Google Meet, Zoom, Teams, and Webex links. Searches:
/// - `hangoutLink` (top level or inside `data`)
/// - `conferenceData.entryPoints[].uri`
/// - `location` field (Zoom/Teams links are often placed here)
/// - recursive fallback across all string values
fn extract_meet_url(payload: &serde_json::Value) -> Option<String> {
    for root in [payload, payload.get("data").unwrap_or(payload)] {
        // hangoutLink (Google Meet)
        if let Some(link) = root.get("hangoutLink").and_then(|v| v.as_str()) {
            if is_meeting_url(link) {
                return Some(link.to_string());
            }
        }

        // conferenceData.entryPoints[].uri
        if let Some(entries) = root
            .get("conferenceData")
            .and_then(|cd| cd.get("entryPoints"))
            .and_then(|ep| ep.as_array())
        {
            for entry in entries {
                if let Some(uri) = entry.get("uri").and_then(|v| v.as_str()) {
                    if is_meeting_url(uri) {
                        return Some(uri.to_string());
                    }
                }
            }
        }

        // location field (Zoom/Teams links are often pasted here)
        if let Some(loc) = root.get("location").and_then(|v| v.as_str()) {
            if is_meeting_url(loc) {
                return Some(loc.to_string());
            }
        }
    }

    // Fallback: scan all string values for any meeting URL.
    find_meet_url_recursive(payload)
}

fn find_meet_url_recursive(val: &serde_json::Value) -> Option<String> {
    match val {
        serde_json::Value::String(s) if is_meeting_url(s) => Some(s.clone()),
        serde_json::Value::Object(map) => {
            for v in map.values() {
                if let Some(url) = find_meet_url_recursive(v) {
                    return Some(url);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for v in arr {
                if let Some(url) = find_meet_url_recursive(v) {
                    return Some(url);
                }
            }
            None
        }
        _ => None,
    }
}

/// Auto-join a meeting via the backend Socket.IO connection.
async fn auto_join_meeting(
    meet_url: String,
    event_title: String,
    correlation_id: String,
    listen_only: bool,
) {
    use crate::openhuman::socket::global_socket_manager;
    use serde_json::json;

    let mgr = match global_socket_manager() {
        Some(mgr) if mgr.is_connected() => mgr,
        _ => {
            tracing::warn!("[meet:calendar] cannot auto-join: socket not connected to backend");
            return;
        }
    };

    let payload = json!({
        "meetUrl": meet_url,
        "displayName": "Tiny",
        "correlationId": correlation_id,
        "listenOnly": listen_only,
    });

    tracing::info!(
        meet_url = %meet_url,
        title = %event_title,
        correlation_id = %correlation_id,
        listen_only = listen_only,
        "[meet:calendar] emitting bot:join"
    );

    if let Err(e) = mgr.emit("bot:join", payload).await {
        tracing::error!(
            error = %e,
            "[meet:calendar] failed to emit bot:join for auto-join"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_hangout_link() {
        let payload = json!({
            "summary": "Standup",
            "hangoutLink": "https://meet.google.com/abc-defg-hij"
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn extracts_nested_hangout_link() {
        let payload = json!({
            "data": {
                "summary": "Standup",
                "hangoutLink": "https://meet.google.com/xyz-abcd-efg"
            }
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://meet.google.com/xyz-abcd-efg")
        );
    }

    #[test]
    fn extracts_from_conference_data() {
        let payload = json!({
            "conferenceData": {
                "entryPoints": [
                    { "entryPointType": "video", "uri": "https://meet.google.com/abc-defg-hij" },
                    { "entryPointType": "phone", "uri": "tel:+1234567890" }
                ]
            }
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn returns_none_when_no_meet_link() {
        let payload = json!({
            "summary": "Lunch",
            "location": "Office kitchen"
        });
        assert!(extract_meet_url(&payload).is_none());
    }

    #[test]
    fn imminent_meeting_starting_in_5_minutes() {
        let start = (chrono::Utc::now() + chrono::Duration::minutes(5)).to_rfc3339();
        let end = (chrono::Utc::now() + chrono::Duration::minutes(35)).to_rfc3339();
        let payload = json!({
            "start": { "dateTime": start },
            "end": { "dateTime": end },
        });
        assert!(is_meeting_imminent(&payload));
    }

    #[test]
    fn not_imminent_meeting_starting_in_2_hours() {
        let start = (chrono::Utc::now() + chrono::Duration::hours(2)).to_rfc3339();
        let end = (chrono::Utc::now() + chrono::Duration::hours(3)).to_rfc3339();
        let payload = json!({
            "start": { "dateTime": start },
            "end": { "dateTime": end },
        });
        assert!(!is_meeting_imminent(&payload));
    }

    #[test]
    fn imminent_meeting_already_started() {
        let start = (chrono::Utc::now() - chrono::Duration::minutes(5)).to_rfc3339();
        let end = (chrono::Utc::now() + chrono::Duration::minutes(25)).to_rfc3339();
        let payload = json!({
            "start": { "dateTime": start },
            "end": { "dateTime": end },
        });
        assert!(is_meeting_imminent(&payload));
    }

    #[test]
    fn not_imminent_meeting_already_ended() {
        let start = (chrono::Utc::now() - chrono::Duration::hours(2)).to_rfc3339();
        let end = (chrono::Utc::now() - chrono::Duration::hours(1)).to_rfc3339();
        let payload = json!({
            "start": { "dateTime": start },
            "end": { "dateTime": end },
        });
        assert!(!is_meeting_imminent(&payload));
    }

    #[test]
    fn imminent_when_no_start_time_fail_open() {
        let payload = json!({ "summary": "Meeting" });
        assert!(is_meeting_imminent(&payload));
    }

    #[test]
    fn imminent_nested_data_start_time() {
        let start = (chrono::Utc::now() + chrono::Duration::minutes(3)).to_rfc3339();
        let payload = json!({
            "data": {
                "start": { "dateTime": start },
            }
        });
        assert!(is_meeting_imminent(&payload));
    }

    #[test]
    fn finds_deeply_nested_meet_url() {
        let payload = json!({
            "data": {
                "nested": {
                    "deep": {
                        "url": "https://meet.google.com/deep-nest-url"
                    }
                }
            }
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://meet.google.com/deep-nest-url")
        );
    }

    #[test]
    fn extracts_zoom_from_location() {
        let payload = json!({
            "summary": "Team sync",
            "location": "https://zoom.us/j/123456789"
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://zoom.us/j/123456789")
        );
    }

    #[test]
    fn extracts_teams_from_conference_data() {
        let payload = json!({
            "conferenceData": {
                "entryPoints": [
                    { "entryPointType": "video", "uri": "https://teams.microsoft.com/l/meetup-join/abc" }
                ]
            }
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/abc")
        );
    }

    #[test]
    fn extracts_webex_recursively() {
        let payload = json!({
            "data": {
                "info": {
                    "link": "https://meet.webex.com/meet/abc"
                }
            }
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://meet.webex.com/meet/abc")
        );
    }
}
