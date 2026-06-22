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
use crate::openhuman::app_state::peek_cached_current_user_identity;
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

        // Resolve the meeting owner (the human the bot should reply to) so the
        // auto-join can pass `respondToParticipant` to the backend bot. The
        // calendar event payload carries the user as a "self" attendee whose
        // displayName matches their Google Meet caption label exactly — the
        // most accurate anchor. Falls back to the signed-in account identity.
        let owner_display_name =
            owner_name_from_event_payload(payload).or_else(fallback_owner_from_account);

        tracing::info!(
            trigger = %trigger,
            meet_url = %meet_url,
            title = %event_title,
            owner_resolved = owner_display_name.is_some(),
            "[meet:calendar] detected imminent Google Meet meeting"
        );

        handle_calendar_meeting_candidate(meet_url, event_title, owner_display_name).await;
    }
}

/// Extract the meeting owner's display name from a Google Calendar event
/// payload — the participant the bot should anchor its replies to.
///
/// Google Calendar marks the connected user's own attendee/organizer/creator
/// record with `self: true`. That record's `displayName` is the same label
/// Google Meet shows in caption regions, so it is the most reliable anchor for
/// the backend bot's `respondToParticipant` gate. Falls back to the local part
/// of the `self` email when no display name is present.
fn owner_name_from_event_payload(payload: &serde_json::Value) -> Option<String> {
    for root in [payload, payload.get("data").unwrap_or(payload)] {
        // attendees[] with self == true
        if let Some(attendees) = root.get("attendees").and_then(|a| a.as_array()) {
            for att in attendees {
                if att.get("self").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(name) = name_or_email_local_part(att) {
                        return Some(name);
                    }
                }
            }
        }

        // organizer / creator with self == true
        for key in ["organizer", "creator"] {
            if let Some(person) = root.get(key) {
                if person.get("self").and_then(|v| v.as_bool()) == Some(true) {
                    if let Some(name) = name_or_email_local_part(person) {
                        return Some(name);
                    }
                }
            }
        }
    }
    None
}

/// Pull a usable display name from a calendar person object: prefer
/// `displayName`, else the local part of `email` (before the `@`).
fn name_or_email_local_part(person: &serde_json::Value) -> Option<String> {
    let trimmed = |v: Option<&serde_json::Value>| -> Option<String> {
        v.and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    if let Some(name) = trimmed(person.get("displayName")) {
        return Some(name);
    }
    let email = trimmed(person.get("email"))?;
    let local = email.split('@').next().unwrap_or(&email).trim();
    if local.is_empty() {
        None
    } else {
        Some(local.to_string())
    }
}

/// Decide the effective `listen_only` mode for an auto-join.
///
/// Reply mode needs a known anchor (the participant the bot replies to). When
/// no anchor resolved we force listen-only — the bot still joins, transcribes,
/// and summarizes as configured, but never speaks — instead of replying to
/// every speaker indiscriminately.
pub(crate) fn effective_listen_only(requested_listen_only: bool, has_anchor: bool) -> bool {
    requested_listen_only || !has_anchor
}

/// Fallback owner identity from the signed-in OpenHuman account when the
/// calendar payload carries no `self` attendee (e.g. heartbeat-polled events
/// that surface only a title + URL). Network-free cache peek.
fn fallback_owner_from_account() -> Option<String> {
    let identity = peek_cached_current_user_identity()?;
    owner_from_identity(identity.name.as_deref(), identity.email.as_deref())
}

/// Pure: derive a reply anchor from an identity's `(name, email)`. Prefers a
/// non-blank name, else the local part of the email. Returns `None` when
/// neither yields a usable value.
fn owner_from_identity(name: Option<&str>, email: Option<&str>) -> Option<String> {
    let clean = |s: Option<&str>| -> Option<String> {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };

    if let Some(name) = clean(name) {
        return Some(name);
    }
    let email = clean(email)?;
    let local = email.split('@').next().unwrap_or(&email).trim();
    if local.is_empty() {
        None
    } else {
        Some(local.to_string())
    }
}

/// Returns `true` when the given policy causes `handle_calendar_meeting_candidate`
/// to publish its own actionable notification — the heartbeat planner uses this
/// to skip the generic "meeting starting" plain card.
///
/// `AskEachTime` surfaces an interactive card (join/skip buttons), so the plain
/// card is redundant. `Always` and `Never` do not surface an interactive card,
/// so the plain card is still useful.
pub(crate) fn auto_join_policy_owns_notification(
    policy: &crate::openhuman::config::schema::AutoJoinPolicy,
) -> bool {
    use crate::openhuman::config::schema::AutoJoinPolicy;
    matches!(policy, AutoJoinPolicy::AskEachTime)
}

/// Apply the user's meeting auto-join policy to a calendar-discovered meeting.
///
/// Returns `true` when this function published (or will publish) its own
/// actionable notification — the heartbeat planner should skip its plain card
/// in that case. Returns `false` for `Always` and `Never` so the caller still
/// fires the generic "meeting starting" reminder.
///
/// This is shared by live Composio calendar triggers and the heartbeat
/// calendar poller. Both sources can discover the same imminent meeting; the
/// Pending-session dedupe below keeps the ask flow to one actionable prompt.
pub async fn handle_calendar_meeting_candidate(
    meet_url: String,
    event_title: String,
    owner_display_name: Option<String>,
) -> bool {
    // Resolve the reply anchor. Callers without payload context (the heartbeat
    // poller passes `None`) fall back to the signed-in account identity here so
    // the bot still knows who to reply to.
    let owner_display_name = owner_display_name
        .or_else(fallback_owner_from_account)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let has_anchor = owner_display_name.is_some();
    if !has_anchor {
        tracing::warn!(
            meet_url = %meet_url,
            "[meet:calendar] no reply anchor resolved — auto-join will fall back to listen-only"
        );
    }

    // Check the auto-join policy.
    let config = match config_rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                error = %e,
                "[meet:calendar] failed to load config, defaulting to ask"
            );
            // Keep the legacy prompt for existing consumers, but return `false`
            // so the heartbeat planner still emits its plain reminder card. We
            // can't build the actionable CoreNotificationEvent here (no config,
            // so no persisted session for the action handler to resolve), and
            // returning `true` would suppress every notification surface —
            // silently "delivering" the meeting with nothing the user can act on.
            publish_global(DomainEvent::MeetAutoJoinPrompt {
                meet_url,
                event_title,
            });
            return false;
        }
    };

    match config.meet.auto_join_policy {
        crate::openhuman::config::schema::AutoJoinPolicy::Never => {
            tracing::debug!("[meet:calendar] auto_join_policy=never, dropping");
            false
        }
        crate::openhuman::config::schema::AutoJoinPolicy::Always => {
            // Dedup: one auto-join per meeting URL while an active session exists.
            // The heartbeat planner can forward the same event from multiple
            // stages (final_call + starting_now) and Composio can re-fire on
            // event updates — without this guard a single meeting generates
            // multiple bot:join calls with distinct correlation IDs that the
            // backend cannot deduplicate.
            if let Ok(Some(existing)) = store::get_session_by_meet_url(&config, &meet_url) {
                if existing.status != MeetingSessionStatus::Ended {
                    tracing::debug!(
                        meeting_id = %existing.id,
                        "[meet:calendar] auto_join_policy=always, open session exists — skipping duplicate join"
                    );
                    return false;
                }
            }

            tracing::info!(
                meet_url = %meet_url,
                title = %event_title,
                "[meet:calendar] auto_join_policy=always, joining automatically"
            );
            let correlation_id = uuid::Uuid::new_v4().to_string();
            // Honor the user's listen-only default (issue #3511 settings
            // UI), but force listen-only when no reply anchor resolved so the
            // bot transcribes/summarizes instead of replying to everyone.
            let listen_only = effective_listen_only(config.meet.listen_only_default, has_anchor);
            if listen_only && !config.meet.listen_only_default {
                tracing::warn!(
                    meet_url = %meet_url,
                    "[meet:calendar] forcing listen-only auto-join (no reply anchor)"
                );
            }

            // Persist a session keyed by correlation_id so future trigger
            // firings find the existing entry and skip (see dedup guard above).
            let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
            let session = MeetingSession {
                id: correlation_id.clone(),
                meet_url: meet_url.clone(),
                title: Some(event_title.clone()),
                calendar_event_id: None,
                status: MeetingSessionStatus::Joined,
                source: AutoJoinSource::Calendar,
                thread_id: None,
                transcript_received: false,
                summary_generated: false,
                created_at_ms: now_ms,
                updated_at_ms: now_ms,
            };
            if let Err(e) = store::create_session(&config, &session) {
                tracing::warn!(
                    error = %e,
                    "[meet:calendar] session create failed for always-join; dedup best-effort only"
                );
            }

            // Auto-join transparency: announce the triggered join so
            // downstream consumers (UI banner, thread bus) can react
            // (issue #3507 contract event).
            publish_global(DomainEvent::MeetingAutoJoinTriggered {
                meeting_id: correlation_id.clone(),
                meet_url: meet_url.clone(),
                listen_only,
                correlation_id: correlation_id.clone(),
            });
            tokio::spawn(auto_join_meeting(
                meet_url,
                event_title,
                correlation_id,
                listen_only,
                owner_display_name,
            ));
            false
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
            // updates; heartbeat can also poll the same event).
            if let Ok(Some(existing)) = store::get_session_by_meet_url(&config, &meet_url) {
                if existing.status != MeetingSessionStatus::Ended {
                    tracing::debug!(
                        meeting_id = %existing.id,
                        "[meet:calendar] open session already exists — skipping re-prompt"
                    );
                    return true;
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
                // The action buttons carry this `meetingId`; the
                // `agent_meetings_notification_action` handler resolves the
                // session by id. If persistence failed there is no session to
                // resolve, so publishing the actionable card would hand the user
                // Join/Skip/Always buttons that fail against missing state.
                // Fall back to the plain reminder path instead.
                tracing::warn!(
                    error = %e,
                    "[meet:calendar] session create failed; falling back to plain reminder without actionable buttons"
                );
                publish_global(DomainEvent::MeetAutoJoinPrompt {
                    meet_url,
                    event_title,
                });
                return false;
            }

            // Announce the new Pending session (issue #3507 contract event).
            publish_global(DomainEvent::MeetingSessionCreated {
                meeting_id: meeting_id.clone(),
                meet_url: meet_url.clone(),
                title: event_title.clone(),
                source: "calendar".to_string(),
            });

            // Carry the resolved reply anchor through the notification buttons so
            // `handle_notification_action` can pass `respondToParticipant` to the
            // backend bot when the user chooses "Join & reply".
            let action_payload = build_action_payload(
                &meeting_id,
                &meet_url,
                &event_title,
                owner_display_name.as_deref(),
            );
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
            true
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

/// Pull the first parseable meeting URL out of a free-form string.
///
/// Calendar `location` is free-form and commonly mixes a label with a URL
/// (e.g. `"Zoom Meeting: https://zoom.us/j/123"`). Returning the raw string
/// would produce a `meeting_url` that `url::Url::parse` later rejects, leaving
/// Join/Skip buttons that silently fail. So scan whitespace-separated tokens,
/// strip surrounding punctuation (including trailing `.`), and return the first
/// token that both matches a known meeting host and parses as an http(s) URL.
fn extract_meeting_url_from_text(text: &str) -> Option<String> {
    text.split_whitespace()
        .map(|tok| {
            tok.trim_matches(|c: char| {
                matches!(
                    c,
                    '(' | ')' | '[' | ']' | '<' | '>' | ',' | ';' | '"' | '\'' | '.'
                )
            })
        })
        .filter(|tok| is_meeting_url(tok))
        .find_map(|tok| {
            let parsed = url::Url::parse(tok).ok()?;
            matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
        })
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

        // location field (Zoom/Teams links are often pasted here as free-form
        // text, e.g. "Zoom Meeting: https://zoom.us/j/123"). Extract only the
        // parseable URL token — returning the whole string would fail later
        // validation in handle_join → validate_meeting_url.
        if let Some(loc) = root.get("location").and_then(|v| v.as_str()) {
            if let Some(url) = extract_meeting_url_from_text(loc) {
                return Some(url);
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
    owner_display_name: Option<String>,
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

    let payload = build_auto_join_payload(
        &meet_url,
        &correlation_id,
        listen_only,
        owner_display_name.as_deref(),
    );

    tracing::info!(
        meet_url = %meet_url,
        title = %event_title,
        correlation_id = %correlation_id,
        listen_only = listen_only,
        respond_to = ?owner_display_name,
        "[meet:calendar] emitting bot:join"
    );

    if let Err(e) = mgr.emit("bot:join", payload).await {
        tracing::error!(
            error = %e,
            "[meet:calendar] failed to emit bot:join for auto-join"
        );
    }
}

/// Build the notification action payload carried by the AskEachTime buttons.
///
/// Pure function so the `respondToParticipant` anchor wiring is unit-testable.
/// A `None`/empty owner omits `respondToParticipant`.
fn build_action_payload(
    meeting_id: &str,
    meet_url: &str,
    title: &str,
    owner_display_name: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "meetingId": meeting_id,
        "meetUrl": meet_url,
        "title": title,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(owner) = owner_display_name.map(str::trim).filter(|s| !s.is_empty()) {
            map.insert("respondToParticipant".to_string(), serde_json::json!(owner));
        }
    }
    payload
}

/// Build the `bot:join` Socket.IO payload for a calendar auto-join.
///
/// Pure function so the `respondToParticipant` anchor wiring is unit-testable
/// without a live socket. A `None`/empty owner omits `respondToParticipant`,
/// which the backend bot treats as "respond to everyone".
fn build_auto_join_payload(
    meet_url: &str,
    correlation_id: &str,
    listen_only: bool,
    owner_display_name: Option<&str>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "meetUrl": meet_url,
        "displayName": "Tiny",
        "correlationId": correlation_id,
        "listenOnly": listen_only,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(owner) = owner_display_name.map(str::trim).filter(|s| !s.is_empty()) {
            map.insert("respondToParticipant".to_string(), serde_json::json!(owner));
        }
    }
    payload
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

    // ── auto_join_policy_owns_notification ──────────────────────

    #[test]
    fn ask_each_time_owns_notification() {
        use crate::openhuman::config::schema::AutoJoinPolicy;
        assert!(auto_join_policy_owns_notification(
            &AutoJoinPolicy::AskEachTime
        ));
    }

    #[test]
    fn always_does_not_own_notification() {
        use crate::openhuman::config::schema::AutoJoinPolicy;
        assert!(!auto_join_policy_owns_notification(&AutoJoinPolicy::Always));
    }

    #[test]
    fn never_does_not_own_notification() {
        use crate::openhuman::config::schema::AutoJoinPolicy;
        assert!(!auto_join_policy_owns_notification(&AutoJoinPolicy::Never));
    }

    // ── owner_name_from_event_payload ───────────────────────────

    #[test]
    fn owner_from_self_attendee_display_name() {
        let payload = json!({
            "summary": "Standup",
            "attendees": [
                { "email": "bob@x.com", "displayName": "Bob" },
                { "email": "me@x.com", "self": true, "displayName": "Aditya L" },
            ]
        });
        assert_eq!(
            owner_name_from_event_payload(&payload).as_deref(),
            Some("Aditya L")
        );
    }

    #[test]
    fn owner_from_self_attendee_email_local_part_when_no_name() {
        let payload = json!({
            "attendees": [
                { "email": "aditya@syvora.com", "self": true },
            ]
        });
        assert_eq!(
            owner_name_from_event_payload(&payload).as_deref(),
            Some("aditya")
        );
    }

    #[test]
    fn owner_from_nested_data_attendees() {
        let payload = json!({
            "data": {
                "attendees": [
                    { "email": "me@x.com", "self": true, "displayName": "Nested Me" },
                ]
            }
        });
        assert_eq!(
            owner_name_from_event_payload(&payload).as_deref(),
            Some("Nested Me")
        );
    }

    #[test]
    fn owner_from_organizer_self() {
        let payload = json!({
            "organizer": { "email": "org@x.com", "self": true, "displayName": "Organizer" }
        });
        assert_eq!(
            owner_name_from_event_payload(&payload).as_deref(),
            Some("Organizer")
        );
    }

    #[test]
    fn owner_none_when_no_self_record() {
        let payload = json!({
            "attendees": [
                { "email": "bob@x.com", "displayName": "Bob" },
            ],
            "organizer": { "email": "org@x.com", "displayName": "Org" }
        });
        assert!(owner_name_from_event_payload(&payload).is_none());
    }

    #[test]
    fn owner_ignores_blank_display_name_falls_to_email() {
        let payload = json!({
            "attendees": [
                { "email": "carol@x.com", "self": true, "displayName": "   " },
            ]
        });
        assert_eq!(
            owner_name_from_event_payload(&payload).as_deref(),
            Some("carol")
        );
    }

    // ── build_auto_join_payload ─────────────────────────────────

    #[test]
    fn auto_join_payload_includes_respond_to_participant() {
        let p = build_auto_join_payload(
            "https://meet.google.com/abc",
            "corr-1",
            false,
            Some("Aditya"),
        );
        assert_eq!(p["respondToParticipant"], json!("Aditya"));
        assert_eq!(p["displayName"], json!("Tiny"));
        assert_eq!(p["listenOnly"], json!(false));
        assert_eq!(p["correlationId"], json!("corr-1"));
    }

    #[test]
    fn auto_join_payload_omits_respond_to_participant_when_absent() {
        let p = build_auto_join_payload("https://meet.google.com/abc", "corr-1", true, None);
        assert!(p.get("respondToParticipant").is_none());
    }

    #[test]
    fn auto_join_payload_omits_respond_to_participant_when_blank() {
        let p = build_auto_join_payload("https://meet.google.com/abc", "corr-1", true, Some("   "));
        assert!(p.get("respondToParticipant").is_none());
    }

    // ── effective_listen_only ───────────────────────────────────

    #[test]
    fn listen_only_forced_when_no_anchor() {
        // Reply requested (listen_only=false) but no anchor → forced listen-only.
        assert!(effective_listen_only(false, false));
    }

    #[test]
    fn reply_mode_kept_when_anchor_present() {
        assert!(!effective_listen_only(false, true));
    }

    #[test]
    fn listen_only_stays_listen_only_regardless_of_anchor() {
        assert!(effective_listen_only(true, true));
        assert!(effective_listen_only(true, false));
    }

    // ── owner_from_identity ─────────────────────────────────────

    #[test]
    fn owner_from_identity_prefers_name() {
        assert_eq!(
            owner_from_identity(Some("Shanu Goyanka"), Some("shanu@x.com")).as_deref(),
            Some("Shanu Goyanka")
        );
    }

    #[test]
    fn owner_from_identity_falls_back_to_email_local_part() {
        assert_eq!(
            owner_from_identity(Some("  "), Some("shanu@x.com")).as_deref(),
            Some("shanu")
        );
        assert_eq!(
            owner_from_identity(None, Some("aditya@syvora.com")).as_deref(),
            Some("aditya")
        );
    }

    #[test]
    fn owner_from_identity_none_when_both_blank() {
        assert!(owner_from_identity(None, None).is_none());
        assert!(owner_from_identity(Some("  "), Some("   ")).is_none());
    }

    // ── build_action_payload ────────────────────────────────────

    #[test]
    fn action_payload_includes_respond_to_participant() {
        let p = build_action_payload(
            "m-1",
            "https://meet.google.com/abc",
            "Standup",
            Some("Shanu Goyanka"),
        );
        assert_eq!(p["meetingId"], json!("m-1"));
        assert_eq!(p["meetUrl"], json!("https://meet.google.com/abc"));
        assert_eq!(p["title"], json!("Standup"));
        assert_eq!(p["respondToParticipant"], json!("Shanu Goyanka"));
    }

    #[test]
    fn action_payload_omits_respond_to_participant_when_absent_or_blank() {
        let p = build_action_payload("m-1", "https://meet.google.com/abc", "Standup", None);
        assert!(p.get("respondToParticipant").is_none());
        let p2 = build_action_payload("m-1", "https://meet.google.com/abc", "Standup", Some("  "));
        assert!(p2.get("respondToParticipant").is_none());
    }

    // ── extract_meeting_url_from_text ───────────────────────────

    #[test]
    fn extracts_url_from_free_form_location_with_label() {
        assert_eq!(
            extract_meeting_url_from_text("Zoom Meeting: https://zoom.us/j/123456789"),
            Some("https://zoom.us/j/123456789".to_string())
        );
    }

    #[test]
    fn strips_surrounding_parens_from_url() {
        assert_eq!(
            extract_meeting_url_from_text("Join here (https://zoom.us/j/999),"),
            Some("https://zoom.us/j/999".to_string())
        );
    }

    #[test]
    fn strips_trailing_period_from_url() {
        assert_eq!(
            extract_meeting_url_from_text("Link: https://zoom.us/j/123."),
            Some("https://zoom.us/j/123".to_string())
        );
    }

    #[test]
    fn returns_none_for_non_meeting_free_form() {
        assert!(extract_meeting_url_from_text("Office kitchen, 2nd floor").is_none());
    }

    #[test]
    fn extracts_zoom_from_free_form_location_field() {
        let payload = json!({
            "summary": "Team sync",
            "location": "Zoom Meeting: https://zoom.us/j/987654321"
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://zoom.us/j/987654321")
        );
    }

    #[test]
    fn extracts_teams_from_free_form_location_field() {
        let payload = json!({
            "summary": "Planning",
            "location": "MS Teams: https://teams.microsoft.com/l/meetup-join/abc"
        });
        assert_eq!(
            extract_meet_url(&payload).as_deref(),
            Some("https://teams.microsoft.com/l/meetup-join/abc")
        );
    }
}
