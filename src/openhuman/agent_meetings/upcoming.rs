//! Fetch upcoming calendar meetings via Composio for the
//! `openhuman.meet_list_upcoming` RPC.
//!
//! Independent of the heartbeat collectors path — two consumers, one Composio
//! access pattern. Does NOT touch the heartbeat planner or notification pipeline.
//!
//! ## Design note
//!
//! The heartbeat's `collect_calendar_events` helper is intentionally NOT shared
//! here (brief guidance: "Do NOT refactor the heartbeat collectors module to
//! share code — that risks regressing the notification planner"). We reuse only
//! the Composio client factory (`create_composio_client`) and the calendar query
//! defaults helper (`apply_calendar_query_defaults`) — both are already `pub` and
//! carry zero heartbeat logic.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde_json::json;

use crate::openhuman::composio::client::{
    create_composio_client, direct_execute, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::types::ComposioConnection;
use crate::openhuman::config::Config;

use super::types::UpcomingMeeting;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub(crate) const DEFAULT_LOOKAHEAD_MINUTES: u32 = 480; // 8 hours
pub(crate) const DEFAULT_LIMIT: u32 = 20;

// URL/host/platform helpers (`is_meeting_url`, `extract_url_from_text`,
// `infer_platform_from_url`) are the canonical strict versions in `super::ops`
// — see finding #9 consolidation. This module no longer carries its own copies.

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Fetch upcoming meetings across all connected Google Calendar accounts.
///
/// Returns an empty `Vec` (not an error) when no calendar is connected —
/// this is expected for users who haven't linked Google Calendar yet.
pub(crate) async fn fetch_upcoming_meetings(
    config: &Config,
    lookahead_minutes: u32,
    limit: u32,
    join_policy: &str,
) -> Result<Vec<UpcomingMeeting>, String> {
    let now = Utc::now();
    let end_window = now + chrono::Duration::minutes(i64::from(lookahead_minutes.max(1)));

    tracing::debug!(
        lookahead_minutes,
        limit,
        now = %now,
        end_window = %end_window,
        "[meet:upcoming] fetch start"
    );

    // Build the mode-aware Composio client. Fails gracefully when the user
    // is not signed in or has no Composio config — same pattern as the
    // heartbeat planner.
    let kind = match create_composio_client(config) {
        Ok(k) => k,
        Err(e) => {
            tracing::info!(
                error = %e,
                "[meet:upcoming] Composio client unavailable — skipping (no calendar connected)"
            );
            return Ok(Vec::new());
        }
    };

    // List connections and filter to active Google Calendar connections only.
    // Propagate API errors — a failed list_connections is not the same as
    // "no connections"; it means we could not determine the connection state.
    let connections = fetch_connections(&kind).await?;
    let calendar_connections: Vec<_> = connections
        .into_iter()
        .filter(|c| c.is_active() && is_calendar_connection(c))
        .collect();

    tracing::debug!(
        count = calendar_connections.len(),
        "[meet:upcoming] active calendar connections"
    );

    if calendar_connections.is_empty() {
        return Ok(Vec::new());
    }

    let mut all_meetings: Vec<UpcomingMeeting> = Vec::new();
    // Global dedup across connections: same event may appear via multiple accounts.
    let mut seen_ids: HashSet<String> = HashSet::new();

    for conn in &calendar_connections {
        // Propagate per-connection fetch errors — an API failure for a
        // connected calendar is not an empty-calendar case and must surface as
        // an error so the UI can show its error state rather than a blank list.
        let events = fetch_events_for_connection(
            &kind,
            conn,
            &config.composio.entity_id,
            now,
            end_window,
            limit,
            join_policy,
            &mut seen_ids,
        )
        .await?;
        all_meetings.extend(events);
    }

    // Sort soonest-first, apply limit.
    all_meetings.sort_by_key(|m| m.start_time_ms);
    all_meetings.truncate(limit as usize);

    tracing::info!(total = all_meetings.len(), "[meet:upcoming] fetch complete");

    Ok(all_meetings)
}

// ---------------------------------------------------------------------------
// Connection helpers
// ---------------------------------------------------------------------------

/// Fetch the full list of Composio connections.
///
/// Returns `Ok(connections)` on success or `Err(message)` on any API /
/// transport failure.  An empty `Ok(vec![])` is a valid response when the
/// user has no connections configured — callers must NOT conflate that with
/// an error.
async fn fetch_connections(kind: &ComposioClientKind) -> Result<Vec<ComposioConnection>, String> {
    match kind {
        ComposioClientKind::Backend(client) => match client.list_connections().await {
            Ok(resp) => {
                tracing::debug!(
                    count = resp.connections.len(),
                    "[meet:upcoming] list_connections (backend) ok"
                );
                Ok(resp.connections)
            }
            Err(e) => {
                tracing::warn!(error = %e, "[meet:upcoming] list_connections (backend) failed");
                Err(format!("[meet:upcoming] list_connections failed: {e}"))
            }
        },
        ComposioClientKind::Direct(direct) => match direct_list_connections(direct).await {
            Ok(resp) => {
                tracing::debug!(
                    count = resp.connections.len(),
                    "[meet:upcoming] list_connections (direct) ok"
                );
                Ok(resp.connections)
            }
            Err(e) => {
                tracing::warn!(error = %e, "[meet:upcoming] list_connections (direct) failed");
                Err(format!("[meet:upcoming] list_connections failed: {e}"))
            }
        },
    }
}

fn is_calendar_connection(conn: &ComposioConnection) -> bool {
    let toolkit = conn.normalized_toolkit();
    toolkit == "googlecalendar" || toolkit == "google_calendar" || toolkit == "calendar"
}

// ---------------------------------------------------------------------------
// Per-connection event fetch
// ---------------------------------------------------------------------------

/// Compute the Google Calendar `maxResults` page size for a fetch.
///
/// Always returns 100 (the API maximum) regardless of the caller's `limit`.
/// Conferencing-link events are only identified AFTER fetching, so passing
/// `limit` as `maxResults` silently drops link-bearing events that fall past
/// position N in the raw calendar page (behind reminders, OOO blocks, etc.).
/// The real cap is applied by `truncate(limit)` in `fetch_upcoming_meetings`
/// once the filter has run.
fn page_size(_limit: u32) -> u32 {
    100
}

/// Fetch calendar events for one Composio connection and extract upcoming
/// meetings with conferencing links.
///
/// Returns `Ok(meetings)` when the API call succeeds (possibly empty when no
/// link-bearing events fall within the window).  Returns `Err(message)` on an
/// API transport failure or when the Google Calendar tool reports
/// `successful = false` — the caller should surface this as an error state
/// rather than silently treating it as an empty calendar.
async fn fetch_events_for_connection(
    kind: &ComposioClientKind,
    conn: &ComposioConnection,
    entity_id: &str,
    now: DateTime<Utc>,
    end_window: DateTime<Utc>,
    limit: u32,
    join_policy: &str,
    seen_ids: &mut HashSet<String>,
) -> Result<Vec<UpcomingMeeting>, String> {
    // Always fetch a full page (100) so that link-bearing events sitting past
    // the first `limit` raw entries (behind reminders, OOO blocks, etc.) are
    // not silently dropped before the conferencing-link filter runs.
    // `timeMin = now` filters on an event's *end* time so in-progress meetings
    // are also returned.
    let max_results = page_size(limit);
    let arguments = json!({
        "connectionId": conn.id,
        "timeMin": now.to_rfc3339(),
        "timeMax": end_window.to_rfc3339(),
        "maxResults": max_results,
    });
    let iana = crate::openhuman::composio::googlecalendar_args::current_iana_timezone();
    let arguments = crate::openhuman::composio::googlecalendar_args::apply_calendar_query_defaults(
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(arguments),
        &iana,
    );

    tracing::debug!(
        connection_id = %conn.id,
        iana = %iana,
        "[meet:upcoming] fetching GOOGLECALENDAR_EVENTS_LIST"
    );

    let resp = match kind {
        ComposioClientKind::Backend(client) => {
            client
                .execute_tool("GOOGLECALENDAR_EVENTS_LIST", arguments)
                .await
        }
        ComposioClientKind::Direct(direct) => {
            direct_execute(
                direct,
                "GOOGLECALENDAR_EVENTS_LIST",
                arguments,
                entity_id,
                None,
            )
            .await
        }
    };

    match resp {
        Ok(r) if r.successful => {
            let events = extract_upcoming_meetings(&r.data, now, end_window, join_policy, seen_ids);
            tracing::debug!(
                connection_id = %conn.id,
                event_count = events.len(),
                "[meet:upcoming] events with conferencing link extracted"
            );
            Ok(events)
        }
        Ok(r) => {
            let detail = r.error.as_deref().unwrap_or("unsuccessful=true");
            tracing::warn!(
                connection_id = %conn.id,
                error = %detail,
                "[meet:upcoming] GOOGLECALENDAR_EVENTS_LIST returned unsuccessful"
            );
            Err(format!(
                "[meet:upcoming] GOOGLECALENDAR_EVENTS_LIST unsuccessful for connection {}: {detail}",
                conn.id
            ))
        }
        Err(e) => {
            tracing::warn!(
                connection_id = %conn.id,
                error = %e,
                "[meet:upcoming] GOOGLECALENDAR_EVENTS_LIST transport error"
            );
            Err(format!(
                "[meet:upcoming] GOOGLECALENDAR_EVENTS_LIST transport error for connection {}: {e}",
                conn.id
            ))
        }
    }
}

// ---------------------------------------------------------------------------
// Event extraction
// ---------------------------------------------------------------------------

fn extract_upcoming_meetings(
    data: &serde_json::Value,
    start_window: DateTime<Utc>,
    end_window: DateTime<Utc>,
    join_policy: &str,
    seen_ids: &mut HashSet<String>,
) -> Vec<UpcomingMeeting> {
    let mut out = Vec::new();
    collect_recursive(
        data,
        start_window,
        end_window,
        join_policy,
        seen_ids,
        &mut out,
    );
    out
}

fn collect_recursive(
    value: &serde_json::Value,
    start_window: DateTime<Utc>,
    end_window: DateTime<Utc>,
    join_policy: &str,
    seen_ids: &mut HashSet<String>,
    out: &mut Vec<UpcomingMeeting>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_recursive(item, start_window, end_window, join_policy, seen_ids, out);
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(meeting) = try_extract_meeting(map, start_window, end_window, join_policy) {
                // Global dedup: skip if this event id was already extracted from
                // a different connection or an earlier part of the same response.
                if seen_ids.insert(meeting.calendar_event_id.clone()) {
                    out.push(meeting);
                }
            }
            // Recurse into children so we handle Composio envelope shapes
            // (e.g. { "items": [...] }) without hardcoding the key name.
            for child in map.values() {
                collect_recursive(child, start_window, end_window, join_policy, seen_ids, out);
            }
        }
        _ => {}
    }
}

/// Try to interpret a JSON object as a Google Calendar event with a conferencing
/// link. Returns `None` if:
/// - the object has no `start.dateTime` (all-day events, metadata objects), or
/// - the start time is outside `[start_window, end_window]`, or
/// - the event has no parseable meeting URL.
fn try_extract_meeting(
    map: &serde_json::Map<String, serde_json::Value>,
    start_window: DateTime<Utc>,
    end_window: DateTime<Utc>,
    join_policy: &str,
) -> Option<UpcomingMeeting> {
    // Only timed events (start.dateTime). All-day events only have start.date.
    let start_str = datetime_field(map, "start", "dateTime")?;
    let start_dt = chrono::DateTime::parse_from_rfc3339(start_str).ok()?;
    let start_utc = start_dt.with_timezone(&Utc);

    // Parse the end time up front (default 1-hour duration when absent) so we
    // can keep meetings that are *currently in progress*.
    let end_utc = datetime_field(map, "end", "dateTime")
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    // Window check that INCLUDES in-progress meetings: keep an event when it
    // hasn't ended yet (end >= now, where `start_window` == now) and it starts
    // before the window end. A meeting that started 10 min ago and ends in
    // 20 min must still appear — the previous `start_utc < start_window` check
    // dropped it.
    let effective_end = end_utc.unwrap_or(start_utc);
    if effective_end < start_window || start_utc > end_window {
        return None;
    }

    let start_ms = start_utc.timestamp_millis().max(0) as u64;

    let end_ms = end_utc
        .map(|dt| dt.timestamp_millis().max(0) as u64)
        .unwrap_or_else(|| start_ms + 3_600_000); // Default 1-hour duration

    // Must have a conferencing URL. This is the filter: events without a
    // meeting link are calendar items (appointments, reminders, OOO) that the
    // Upcoming table doesn't need to show.
    let meet_url = extract_meet_url_from_event(map)?;

    let platform = super::ops::infer_platform_from_url(&meet_url).map(str::to_string);

    let title = map
        .get("summary")
        .or_else(|| map.get("title"))
        .or_else(|| map.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled meeting")
        .to_string();

    // Stable event id for dedup + per-event policy keying. When the event has no
    // real id (id/eventId/icalUID all absent), fall back to a STABLE synthetic
    // key from the meeting URL + start time rather than a shared literal
    // "unknown" — otherwise every id-less event collapses onto the same dedup
    // key and only the first survives.
    let calendar_event_id = super::ops::extract_calendar_event_id(map)
        .unwrap_or_else(|| format!("{meet_url}@{start_ms}"));

    let participant_count = map
        .get("attendees")
        .and_then(|a| a.as_array())
        .map(|arr| arr.len() as u32);

    let organizer = map
        .get("organizer")
        .and_then(|o| {
            o.get("displayName")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .or_else(|| o.get("email").and_then(|v| v.as_str()))
        })
        .map(str::to_string);

    tracing::debug!(
        calendar_event_id = %calendar_event_id,
        title = %title,
        start_ms,
        platform = ?platform,
        "[meet:upcoming] extracted meeting with conferencing link"
    );

    Some(UpcomingMeeting {
        calendar_event_id,
        title,
        start_time_ms: start_ms,
        end_time_ms: end_ms,
        meet_url: Some(meet_url),
        platform,
        participant_count,
        organizer,
        join_policy: join_policy.to_string(),
        calendar_source: "googlecalendar".to_string(),
    })
}

// ---------------------------------------------------------------------------
// Event URL extraction
//
// The strict URL/host/platform primitives (`is_meeting_url`,
// `extract_url_from_text`, `infer_platform_from_url`) live in `super::ops` —
// this module just composes them over the Google Calendar event shape.
// ---------------------------------------------------------------------------

/// Extract the conferencing URL from a Google Calendar event object, checking
/// in priority order:
///
/// 1. `hangoutLink` (Google Meet direct field)
/// 2. `conferenceData.entryPoints[].uri`
/// 3. `location` field (Zoom/Teams links often pasted here as free-form text)
/// 4. `description` field (fallback — some invites embed the link in the body)
fn extract_meet_url_from_event(map: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    // 1. hangoutLink
    if let Some(link) = map.get("hangoutLink").and_then(|v| v.as_str()) {
        if super::ops::is_meeting_url(link) {
            return Some(link.to_string());
        }
    }
    // 2. conferenceData.entryPoints[].uri
    if let Some(entries) = map
        .get("conferenceData")
        .and_then(|cd| cd.get("entryPoints"))
        .and_then(|ep| ep.as_array())
    {
        for entry in entries {
            if let Some(uri) = entry.get("uri").and_then(|v| v.as_str()) {
                if super::ops::is_meeting_url(uri) {
                    return Some(uri.to_string());
                }
            }
        }
    }
    // 3. location (free-form, e.g. "Zoom Meeting: https://zoom.us/j/123")
    if let Some(loc) = map.get("location").and_then(|v| v.as_str()) {
        if let Some(url) = super::ops::extract_url_from_text(loc) {
            return Some(url);
        }
    }
    // 4. description (last resort)
    if let Some(desc) = map.get("description").and_then(|v| v.as_str()) {
        if let Some(url) = super::ops::extract_url_from_text(desc) {
            return Some(url);
        }
    }
    None
}

/// Pull a `start.dateTime` (or `end.dateTime`) string from an event map.
fn datetime_field<'a>(
    map: &'a serde_json::Map<String, serde_json::Value>,
    outer: &str,
    inner: &str,
) -> Option<&'a str> {
    map.get(outer)
        .and_then(|v| v.as_object())
        .and_then(|o| o.get(inner))
        .and_then(|v| v.as_str())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent_meetings::ops::infer_platform_from_url;
    use serde_json::json;

    fn window() -> (DateTime<Utc>, DateTime<Utc>) {
        let now = Utc::now();
        let end = now + chrono::Duration::hours(8);
        (now, end)
    }

    fn future_event(offset_minutes: i64) -> serde_json::Value {
        let start = Utc::now() + chrono::Duration::minutes(offset_minutes);
        let end = start + chrono::Duration::hours(1);
        json!({
            "id": format!("event-{offset_minutes}"),
            "summary": format!("Meeting in {offset_minutes}m"),
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "hangoutLink": "https://meet.google.com/abc-defg-hij"
        })
    }

    // ── event JSON → UpcomingMeeting mapping ──────────────────────

    #[test]
    fn extracts_meeting_from_event_with_hangout_link() {
        let (start_window, end_window) = window();
        let event = future_event(30);
        let map = event.as_object().unwrap();
        let meeting = try_extract_meeting(map, start_window, end_window, "ask").unwrap();
        assert_eq!(meeting.title, "Meeting in 30m");
        assert_eq!(
            meeting.meet_url.as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(meeting.platform.as_deref(), Some("gmeet"));
        assert_eq!(meeting.join_policy, "ask");
        assert_eq!(meeting.calendar_source, "googlecalendar");
    }

    #[test]
    fn extracts_meeting_from_conference_data_entry_points() {
        let (start_window, end_window) = window();
        let start = Utc::now() + chrono::Duration::minutes(60);
        let end = start + chrono::Duration::hours(1);
        let event = json!({
            "id": "ev-1",
            "summary": "Zoom call",
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "conferenceData": {
                "entryPoints": [
                    { "entryPointType": "video", "uri": "https://zoom.us/j/123456789" }
                ]
            }
        });
        let map = event.as_object().unwrap();
        let meeting = try_extract_meeting(map, start_window, end_window, "ask").unwrap();
        assert_eq!(meeting.platform.as_deref(), Some("zoom"));
        assert_eq!(
            meeting.meet_url.as_deref(),
            Some("https://zoom.us/j/123456789")
        );
    }

    #[test]
    fn skips_event_with_no_conferencing_link() {
        let (start_window, end_window) = window();
        let start = Utc::now() + chrono::Duration::minutes(60);
        let end = start + chrono::Duration::hours(1);
        let event = json!({
            "id": "ev-noop",
            "summary": "Lunch",
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "location": "Office kitchen"
        });
        let map = event.as_object().unwrap();
        assert!(try_extract_meeting(map, start_window, end_window, "ask").is_none());
    }

    #[test]
    fn skips_all_day_event_without_datetime() {
        let (start_window, end_window) = window();
        let event = json!({
            "id": "ev-allday",
            "summary": "Holiday",
            "start": { "date": "2026-06-30" },
            "end": { "date": "2026-07-01" },
            "hangoutLink": "https://meet.google.com/xyz"
        });
        let map = event.as_object().unwrap();
        assert!(try_extract_meeting(map, start_window, end_window, "ask").is_none());
    }

    #[test]
    fn skips_event_outside_window() {
        let (start_window, end_window) = window();
        let far_future = Utc::now() + chrono::Duration::hours(24);
        let end = far_future + chrono::Duration::hours(1);
        let event = json!({
            "id": "ev-far",
            "summary": "Future meeting",
            "start": { "dateTime": far_future.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "hangoutLink": "https://meet.google.com/abc"
        });
        let map = event.as_object().unwrap();
        assert!(try_extract_meeting(map, start_window, end_window, "ask").is_none());
    }

    #[test]
    fn page_size_always_uses_api_max() {
        // page_size must always return 100 (the API maximum) so that
        // link-bearing events that fall past the first `limit` raw calendar
        // entries (buried behind reminders, OOO blocks, etc.) survive the
        // conferencing-link filter. The actual limit is applied via truncate()
        // in fetch_upcoming_meetings after filtering.
        assert_eq!(page_size(1), 100);
        assert_eq!(page_size(5), 100);
        assert_eq!(page_size(20), 100);
        assert_eq!(page_size(100), 100);
    }

    #[test]
    fn link_event_past_first_n_raw_entries_is_returned() {
        // Core guarantee of finding #1: a link-bearing event sitting behind
        // 3 link-free events must still be collected even when limit=1.
        // Before the fix, page_size(1)=1 meant only the first raw entry was
        // fetched and the link event was never seen.
        let (start_window, end_window) = window();
        let no_link = |offset: i64, id: &str| {
            let start = Utc::now() + chrono::Duration::minutes(offset);
            let end = start + chrono::Duration::hours(1);
            json!({
                "id": id,
                "summary": format!("No link {offset}m"),
                "start": { "dateTime": start.to_rfc3339() },
                "end": { "dateTime": end.to_rfc3339() },
                "location": "Office"
            })
        };
        let link_start = Utc::now() + chrono::Duration::minutes(45);
        let link_end = link_start + chrono::Duration::hours(1);
        let link_event = json!({
            "id": "link-ev-1",
            "summary": "Zoom sync",
            "start": { "dateTime": link_start.to_rfc3339() },
            "end": { "dateTime": link_end.to_rfc3339() },
            "hangoutLink": "https://meet.google.com/abc-defg-hij"
        });
        // 3 link-free events, then 1 link event (position 4 in a raw page).
        let events = json!([
            no_link(10, "nl-1"),
            no_link(20, "nl-2"),
            no_link(30, "nl-3"),
            link_event
        ]);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        collect_recursive(
            &events,
            start_window,
            end_window,
            "ask",
            &mut seen,
            &mut out,
        );
        assert_eq!(out.len(), 1, "link event at position 4 must survive filter");
        assert_eq!(out[0].calendar_event_id, "link-ev-1");
    }

    #[tokio::test]
    async fn fetch_upcoming_returns_empty_when_no_composio_config() {
        // When no Composio API key or backend URL is configured,
        // create_composio_client fails gracefully → fetch_upcoming_meetings
        // returns Ok([]) (no calendar = empty, not an error).
        let config = crate::openhuman::config::Config::default();
        let result = fetch_upcoming_meetings(&config, 60, 10, "ask").await;
        assert!(
            result.is_ok(),
            "missing Composio config must not be an error: {result:?}"
        );
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn includes_in_progress_meeting() {
        // Started 10 min ago, ends in 20 min. `start_window` == now, so the old
        // `start_utc < start_window` rule dropped this; it must now be kept.
        let (start_window, end_window) = window();
        let start = Utc::now() - chrono::Duration::minutes(10);
        let end = Utc::now() + chrono::Duration::minutes(20);
        let event = json!({
            "id": "ev-inprogress",
            "summary": "Already running",
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "hangoutLink": "https://meet.google.com/in-progress"
        });
        let map = event.as_object().unwrap();
        let meeting = try_extract_meeting(map, start_window, end_window, "ask").unwrap();
        assert_eq!(meeting.title, "Already running");
        assert_eq!(meeting.calendar_event_id, "ev-inprogress");
    }

    #[test]
    fn skips_already_ended_meeting() {
        // Started 2h ago, ended 1h ago — must be dropped.
        let (start_window, end_window) = window();
        let start = Utc::now() - chrono::Duration::hours(2);
        let end = Utc::now() - chrono::Duration::hours(1);
        let event = json!({
            "id": "ev-ended",
            "summary": "Done",
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "hangoutLink": "https://meet.google.com/ended"
        });
        let map = event.as_object().unwrap();
        assert!(try_extract_meeting(map, start_window, end_window, "ask").is_none());
    }

    #[test]
    fn id_less_events_stay_distinct() {
        // Two events with NO id/eventId/icalUID but different URLs + start times
        // must each get a distinct synthetic key and both survive dedup — the
        // old literal "unknown" fallback collapsed them to one.
        let (start_window, end_window) = window();
        let s1 = Utc::now() + chrono::Duration::minutes(15);
        let s2 = Utc::now() + chrono::Duration::minutes(45);
        let ev1 = json!({
            "summary": "Anon A",
            "start": { "dateTime": s1.to_rfc3339() },
            "end": { "dateTime": (s1 + chrono::Duration::hours(1)).to_rfc3339() },
            "hangoutLink": "https://meet.google.com/anon-a"
        });
        let ev2 = json!({
            "summary": "Anon B",
            "start": { "dateTime": s2.to_rfc3339() },
            "end": { "dateTime": (s2 + chrono::Duration::hours(1)).to_rfc3339() },
            "hangoutLink": "https://meet.google.com/anon-b"
        });
        let events = json!([ev1, ev2]);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        collect_recursive(
            &events,
            start_window,
            end_window,
            "ask",
            &mut seen,
            &mut out,
        );
        assert_eq!(out.len(), 2, "id-less events should not collapse");
        // Synthetic keys are URL@startMs and must differ.
        assert_ne!(out[0].calendar_event_id, out[1].calendar_event_id);
        assert!(out[0].calendar_event_id.contains('@'));
    }

    #[test]
    fn id_less_duplicate_event_still_dedups() {
        // The SAME id-less event appearing twice must still collapse to one
        // (stable synthetic key on URL + start time).
        let (start_window, end_window) = window();
        let s = Utc::now() + chrono::Duration::minutes(20);
        let ev = json!({
            "summary": "Anon dup",
            "start": { "dateTime": s.to_rfc3339() },
            "end": { "dateTime": (s + chrono::Duration::hours(1)).to_rfc3339() },
            "hangoutLink": "https://meet.google.com/anon-dup"
        });
        let events = json!([ev, ev]);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        collect_recursive(
            &events,
            start_window,
            end_window,
            "ask",
            &mut seen,
            &mut out,
        );
        assert_eq!(out.len(), 1);
    }

    // ── platform inference ────────────────────────────────────────

    #[test]
    fn infers_platform_from_gmeet() {
        assert_eq!(
            infer_platform_from_url("https://meet.google.com/abc-defg-hij"),
            Some("gmeet")
        );
    }

    #[test]
    fn infers_platform_from_zoom() {
        assert_eq!(
            infer_platform_from_url("https://zoom.us/j/123456789"),
            Some("zoom")
        );
        assert_eq!(
            infer_platform_from_url("https://company.zoom.us/j/123"),
            Some("zoom")
        );
    }

    #[test]
    fn infers_platform_from_teams() {
        assert_eq!(
            infer_platform_from_url("https://teams.microsoft.com/l/meetup-join/abc"),
            Some("teams")
        );
    }

    #[test]
    fn infers_platform_from_webex() {
        assert_eq!(
            infer_platform_from_url("https://meet.webex.com/meet/abc"),
            Some("webex")
        );
    }

    #[test]
    fn infers_platform_none_for_unknown_url() {
        assert!(infer_platform_from_url("https://example.com/meeting").is_none());
    }

    // ── location field extraction ─────────────────────────────────

    #[test]
    fn extracts_zoom_from_location_field() {
        let (start_window, end_window) = window();
        let start = Utc::now() + chrono::Duration::minutes(30);
        let end = start + chrono::Duration::hours(1);
        let event = json!({
            "id": "ev-zoom",
            "summary": "Zoom sync",
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "location": "Zoom Meeting: https://zoom.us/j/987654321"
        });
        let map = event.as_object().unwrap();
        let meeting = try_extract_meeting(map, start_window, end_window, "ask").unwrap();
        assert_eq!(
            meeting.meet_url.as_deref(),
            Some("https://zoom.us/j/987654321")
        );
        assert_eq!(meeting.platform.as_deref(), Some("zoom"));
    }

    // ── lookahead / limit / sort ──────────────────────────────────

    #[test]
    fn extract_respects_window_and_sorts_by_start() {
        let (start_window, end_window) = window();
        let events = json!([future_event(120), future_event(30), future_event(60),]);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        collect_recursive(
            &events,
            start_window,
            end_window,
            "ask",
            &mut seen,
            &mut out,
        );
        // All three are within the 8-hour window.
        assert_eq!(out.len(), 3);
        // They should NOT be sorted here (sorting is done in fetch_upcoming_meetings),
        // but IDs should match what we inserted.
        let ids: Vec<_> = out.iter().map(|m| m.calendar_event_id.as_str()).collect();
        // All ids should be present (order is input order for unsorted collection).
        for id in ["event-120", "event-30", "event-60"] {
            assert!(ids.contains(&id), "missing id: {id}");
        }
    }

    #[test]
    fn deduplicates_events_with_same_id() {
        let (start_window, end_window) = window();
        let event = future_event(30);
        let events = json!([event, event]);
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        collect_recursive(
            &events,
            start_window,
            end_window,
            "ask",
            &mut seen,
            &mut out,
        );
        // Same id should only appear once.
        assert_eq!(out.len(), 1);
    }

    // ── attendee count and organizer ──────────────────────────────

    #[test]
    fn extracts_attendee_count_and_organizer() {
        let (start_window, end_window) = window();
        let start = Utc::now() + chrono::Duration::minutes(30);
        let end = start + chrono::Duration::hours(1);
        let event = json!({
            "id": "ev-people",
            "summary": "Team standup",
            "start": { "dateTime": start.to_rfc3339() },
            "end": { "dateTime": end.to_rfc3339() },
            "hangoutLink": "https://meet.google.com/xyz-abcd",
            "attendees": [
                { "email": "alice@x.com" },
                { "email": "bob@x.com" },
                { "email": "carol@x.com" }
            ],
            "organizer": { "email": "alice@x.com", "displayName": "Alice" }
        });
        let map = event.as_object().unwrap();
        let meeting = try_extract_meeting(map, start_window, end_window, "ask").unwrap();
        assert_eq!(meeting.participant_count, Some(3));
        assert_eq!(meeting.organizer.as_deref(), Some("Alice"));
    }
}
