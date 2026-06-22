use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use crate::openhuman::composio::client::{
    create_composio_client, direct_execute, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::types::{ComposioConnection, ComposioExecuteResponse};
use crate::openhuman::config::Config;
use crate::openhuman::cron;
use crate::openhuman::notifications::store as notifications_store;

use super::types::{HeartbeatCategory, PendingEvent};
use super::utils::{compute_overlap_key, sanitize_preview, stable_key};

pub(crate) fn collect_cron_reminders(config: &Config, now: DateTime<Utc>) -> Vec<PendingEvent> {
    let lookahead = Duration::minutes(i64::from(
        config.heartbeat.reminder_lookahead_minutes.max(1),
    ));

    let jobs = match cron::list_jobs(config) {
        Ok(jobs) => jobs,
        Err(error) => {
            tracing::warn!(error = %error, "[heartbeat:planner] cron list_jobs failed");
            return Vec::new();
        }
    };

    jobs.into_iter()
        .filter(|job| job.enabled)
        .filter(|job| is_reminder_like_job(job))
        .filter(|job| {
            let delta = job.next_run.signed_duration_since(now);
            delta <= lookahead && delta >= Duration::minutes(-2)
        })
        .map(|job| {
            let title = job
                .name
                .clone()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Reminder".to_string());
            let fingerprint = stable_key(&format!("cron:{}:{}", job.id, job.next_run.to_rfc3339()));
            let body = format!(
                "{} is scheduled at {}.",
                title,
                job.next_run.format("%H:%M")
            );

            PendingEvent {
                category: HeartbeatCategory::Reminders,
                source: "cron".to_string(),
                source_event_id: job.id,
                overlap_key: compute_overlap_key(
                    HeartbeatCategory::Reminders,
                    &title,
                    job.next_run,
                ),
                fingerprint,
                title,
                body,
                deep_link: Some("/settings/cron-jobs".to_string()),
                meeting_url: None,
                anchor_at: job.next_run,
            }
        })
        .collect()
}

fn is_reminder_like_job(job: &cron::CronJob) -> bool {
    if job.delivery.mode.eq_ignore_ascii_case("proactive") {
        return true;
    }

    let mut haystack = String::new();
    if let Some(name) = &job.name {
        haystack.push_str(name);
        haystack.push(' ');
    }
    if let Some(prompt) = &job.prompt {
        haystack.push_str(prompt);
        haystack.push(' ');
    }
    haystack.push_str(&job.command);

    let lowered = haystack.to_ascii_lowercase();
    lowered.contains("remind")
        || lowered.contains("meeting")
        || lowered.contains("standup")
        || lowered.contains("follow up")
}

fn is_calendar_connection(connection: &ComposioConnection) -> bool {
    if !connection.is_active() {
        return false;
    }

    let toolkit = connection.normalized_toolkit();
    toolkit == "googlecalendar" || toolkit == "google_calendar" || toolkit == "calendar"
}

fn select_calendar_connections_for_tick(
    connections: Vec<ComposioConnection>,
    limit: usize,
    now: DateTime<Utc>,
    interval_minutes: u32,
) -> Vec<ComposioConnection> {
    let eligible: Vec<_> = connections
        .into_iter()
        .filter(is_calendar_connection)
        .collect();
    let eligible_count = eligible.len();
    let selected_count = eligible_count.min(limit.max(1));

    if selected_count == 0 {
        tracing::debug!(
            target: "composio",
            eligible = eligible_count,
            cap = limit.max(1),
            selected = 0,
            "[heartbeat:planner] calendar-fanout: eligible=0 cap={} selected=0",
            limit.max(1)
        );
        return Vec::new();
    }

    let interval_seconds = i64::from(interval_minutes.max(5)) * 60;
    let tick_index = now.timestamp().div_euclid(interval_seconds);
    let offset = tick_index.rem_euclid(eligible_count as i64) as usize;
    let selected = eligible
        .iter()
        .cycle()
        .skip(offset)
        .take(selected_count)
        .cloned()
        .collect::<Vec<_>>();

    tracing::debug!(
        target: "composio",
        eligible = eligible_count,
        cap = limit.max(1),
        selected = selected_count,
        offset,
        "[heartbeat:planner] calendar-fanout: eligible={} cap={} selected={}",
        eligible_count,
        limit.max(1),
        selected_count
    );

    selected
}

pub(crate) async fn collect_calendar_meetings(
    config: &Config,
    now: DateTime<Utc>,
) -> Vec<PendingEvent> {
    // Route through the mode-aware factory so the heartbeat planner
    // sees the user's *own* Google Calendar connection in direct mode
    // — not the tinyhumans backend tenant's (#1710). Pre-fix, the
    // collector hard-bound to `build_composio_client` (backend-only)
    // and silently returned an empty meeting list for direct-mode
    // users.
    let kind = match create_composio_client(config) {
        Ok(kind) => kind,
        Err(error) => {
            tracing::debug!(
                error = %error,
                "[heartbeat:planner] composio client unavailable — skipping calendar collection"
            );
            return Vec::new();
        }
    };
    tracing::debug!(
        mode = %config.composio.mode,
        "[heartbeat:planner] composio client resolved for calendar collection"
    );

    let connections = match &kind {
        ComposioClientKind::Backend(client) => match client.list_connections().await {
            Ok(resp) => {
                tracing::debug!(
                    count = resp.connections.len(),
                    "[heartbeat:planner] composio list_connections (backend) fetched"
                );
                resp.connections
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "[heartbeat:planner] composio list_connections (backend) failed"
                );
                return Vec::new();
            }
        },
        ComposioClientKind::Direct(direct) => match direct_list_connections(direct).await {
            Ok(resp) => {
                tracing::debug!(
                    count = resp.connections.len(),
                    "[heartbeat:planner] composio list_connections (direct) fetched"
                );
                resp.connections
            }
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "[heartbeat:planner] composio list_connections (direct) failed"
                );
                return Vec::new();
            }
        },
    };

    let lookahead = Duration::minutes(i64::from(config.heartbeat.meeting_lookahead_minutes.max(1)));
    let end_window = now + lookahead;

    let mut out = Vec::new();
    let calendar_connection_limit =
        config.heartbeat.max_calendar_connections_per_tick.max(1) as usize;
    for conn in select_calendar_connections_for_tick(
        connections,
        calendar_connection_limit,
        now,
        config.heartbeat.interval_minutes,
    ) {
        let toolkit = conn.normalized_toolkit();

        // Build base args, then let the shared transformer fill in
        // `timeZone` + `singleEvents` so this poller behaves identically
        // to the agent-driven dispatcher path (issue #1714). Routing
        // both call sites through the same helper means a future change
        // to the defaulting policy only has to land in one place.
        let arguments = json!({
            "connectionId": conn.id,
            "timeMin": now.to_rfc3339(),
            "timeMax": end_window.to_rfc3339(),
            "maxResults": 20
        });
        let iana = crate::openhuman::composio::googlecalendar_args::current_iana_timezone();
        tracing::debug!(
            target: "composio",
            slug = "GOOGLECALENDAR_EVENTS_LIST",
            toolkit = %toolkit,
            connection_id = %conn.id,
            iana = %iana,
            lookahead_minutes = config.heartbeat.meeting_lookahead_minutes.max(1),
            "[composio][heartbeat-planner] applying calendar query defaults pre-poll"
        );
        let arguments =
            crate::openhuman::composio::googlecalendar_args::apply_calendar_query_defaults(
                "GOOGLECALENDAR_EVENTS_LIST",
                Some(arguments),
                &iana,
            );

        let resp: ComposioExecuteResponse = match &kind {
            ComposioClientKind::Backend(client) => {
                match client
                    .execute_tool("GOOGLECALENDAR_EVENTS_LIST", arguments)
                    .await
                {
                    Ok(resp) => resp,
                    Err(error) => {
                        tracing::warn!(
                            toolkit = %toolkit,
                            connection_id = %conn.id,
                            error = %error,
                            "[heartbeat:planner] GOOGLECALENDAR_EVENTS_LIST (backend) failed"
                        );
                        continue;
                    }
                }
            }
            ComposioClientKind::Direct(direct) => {
                match direct_execute(
                    direct,
                    "GOOGLECALENDAR_EVENTS_LIST",
                    arguments,
                    &config.composio.entity_id,
                    None,
                )
                .await
                {
                    Ok(resp) => resp,
                    Err(error) => {
                        tracing::warn!(
                            toolkit = %toolkit,
                            connection_id = %conn.id,
                            error = %error,
                            "[heartbeat:planner] GOOGLECALENDAR_EVENTS_LIST (direct) failed"
                        );
                        continue;
                    }
                }
            }
        };

        out.extend(extract_calendar_events(
            &resp.data, &toolkit, &conn.id, now, end_window,
        ));
    }

    out
}

pub(crate) fn extract_calendar_events(
    value: &serde_json::Value,
    toolkit: &str,
    connection_id: &str,
    start_window: DateTime<Utc>,
    end_window: DateTime<Utc>,
) -> Vec<PendingEvent> {
    let mut out = Vec::new();
    collect_calendar_events_recursive(
        value,
        toolkit,
        connection_id,
        start_window,
        end_window,
        &mut out,
    );
    out
}

fn collect_calendar_events_recursive(
    value: &serde_json::Value,
    toolkit: &str,
    connection_id: &str,
    start_window: DateTime<Utc>,
    end_window: DateTime<Utc>,
    out: &mut Vec<PendingEvent>,
) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                collect_calendar_events_recursive(
                    item,
                    toolkit,
                    connection_id,
                    start_window,
                    end_window,
                    out,
                );
            }
        }
        serde_json::Value::Object(map) => {
            if let Some(starts_at) = extract_datetime_from_map(map) {
                if starts_at >= start_window && starts_at <= end_window {
                    let title = extract_title_from_map(map);
                    let source_event_id = map
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| map.get("eventId").and_then(serde_json::Value::as_str))
                        .or_else(|| map.get("icalUID").and_then(serde_json::Value::as_str))
                        .unwrap_or("calendar-event")
                        .to_string();
                    let deep_link = map
                        .get("htmlLink")
                        .and_then(serde_json::Value::as_str)
                        .or_else(|| map.get("hangoutLink").and_then(serde_json::Value::as_str))
                        .map(ToString::to_string);
                    let meeting_url = extract_meeting_url_from_map(map);

                    let fingerprint = stable_key(&format!(
                        "{}:{}:{}:{}",
                        toolkit,
                        connection_id,
                        source_event_id,
                        starts_at.to_rfc3339()
                    ));

                    out.push(PendingEvent {
                        category: HeartbeatCategory::Meetings,
                        source: format!("calendar:{toolkit}"),
                        source_event_id,
                        overlap_key: compute_overlap_key(
                            HeartbeatCategory::Meetings,
                            &title,
                            starts_at,
                        ),
                        fingerprint,
                        title: title.clone(),
                        body: format!("{} starts at {}.", title, starts_at.format("%H:%M")),
                        deep_link,
                        meeting_url,
                        anchor_at: starts_at,
                    });
                }
            }

            for child in map.values() {
                collect_calendar_events_recursive(
                    child,
                    toolkit,
                    connection_id,
                    start_window,
                    end_window,
                    out,
                );
            }
        }
        _ => {}
    }
}

fn extract_datetime_from_map(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<DateTime<Utc>> {
    // Only accept `start.dateTime` — never fall back to `start.date`.
    // All-day events (birthdays, OOO, holidays) only have a `start.date` field
    // and must not be surfaced as timed meetings.
    let start = map.get("start").and_then(|start| match start {
        serde_json::Value::Object(start_map) => start_map
            .get("dateTime")
            .and_then(serde_json::Value::as_str),
        serde_json::Value::String(s) => Some(s.as_str()),
        _ => None,
    });

    let direct = start
        .or_else(|| map.get("start_time").and_then(serde_json::Value::as_str))
        .or_else(|| map.get("startTime").and_then(serde_json::Value::as_str))
        .or_else(|| map.get("starts_at").and_then(serde_json::Value::as_str))
        .or_else(|| map.get("startsAt").and_then(serde_json::Value::as_str));

    direct.and_then(parse_datetime)
}

fn extract_title_from_map(map: &serde_json::Map<String, serde_json::Value>) -> String {
    map.get("summary")
        .and_then(serde_json::Value::as_str)
        .or_else(|| map.get("title").and_then(serde_json::Value::as_str))
        .or_else(|| map.get("name").and_then(serde_json::Value::as_str))
        .map(|raw| sanitize_preview(raw, 80))
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "Upcoming meeting".to_string())
}

const MEETING_HOST_PATTERNS: &[&str] = &[
    "meet.google.com",
    "zoom.us",
    "teams.microsoft.com",
    "webex.com",
];

fn is_meeting_url(raw: &str) -> bool {
    MEETING_HOST_PATTERNS.iter().any(|pat| raw.contains(pat))
}

/// Pull the first parseable meeting URL out of a free-form string.
///
/// Calendar `location` is free-form and commonly mixes a label with a URL
/// (e.g. `Zoom Meeting: https://zoom.us/j/123`). Returning the whole string
/// would produce a `meeting_url` that the join handler's `url::Url::parse`
/// later rejects, leaving AskEachTime prompts with buttons that always fail
/// while the generic reminder stays suppressed. So scan tokens for one that
/// both matches a known meeting host and parses as an http(s) URL.
fn extract_meeting_url_from_text(text: &str) -> Option<String> {
    text.split_whitespace()
        // Strip surrounding punctuation that often hugs a URL in prose:
        // "(https://zoom.us/j/123)," -> "https://zoom.us/j/123".
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

fn extract_meeting_url_from_map(
    map: &serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    map.get("hangoutLink")
        .and_then(serde_json::Value::as_str)
        .filter(|url| is_meeting_url(url))
        .map(ToString::to_string)
        .or_else(|| {
            map.get("conferenceData")
                .and_then(|cd| cd.get("entryPoints"))
                .and_then(serde_json::Value::as_array)
                .and_then(|entries| {
                    entries.iter().find_map(|entry| {
                        entry
                            .get("uri")
                            .and_then(serde_json::Value::as_str)
                            .filter(|url| is_meeting_url(url))
                            .map(ToString::to_string)
                    })
                })
        })
        .or_else(|| {
            map.get("location")
                .and_then(serde_json::Value::as_str)
                .and_then(extract_meeting_url_from_text)
        })
}

fn parse_datetime(raw: &str) -> Option<DateTime<Utc>> {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
}

pub(crate) fn collect_relevant_notifications(
    config: &Config,
    now: DateTime<Utc>,
) -> Vec<PendingEvent> {
    // Do not apply an importance_score threshold here — urgent and action-worthy
    // notifications may have a low or absent score. The downstream triage_action
    // and raw_payload.urgent checks are the real gate.
    let items = match notifications_store::list(config, 100, 0, None, None) {
        Ok(items) => items,
        Err(error) => {
            tracing::warn!(error = %error, "[heartbeat:planner] notifications list failed");
            return Vec::new();
        }
    };

    items
        .into_iter()
        // Never re-escalate notifications we generated ourselves — that creates a
        // feedback loop where each heartbeat tick spawns a new "Important event"
        // with a fresh ID that bypasses the dedupe store.
        .filter(|item| item.provider != "heartbeat")
        .filter(|item| {
            item.status == crate::openhuman::notifications::types::NotificationStatus::Unread
        })
        .filter(|item| {
            item.triage_action
                .as_deref()
                .map(|action| action == "escalate" || action == "react")
                .unwrap_or(false)
                || item
                    .raw_payload
                    .get("urgent")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false)
        })
        .filter(|item| now.signed_duration_since(item.received_at) <= Duration::minutes(30))
        .map(|item| {
            let title = format!("Important event from {}", item.provider);
            let body = sanitize_preview(&item.title, 100);

            PendingEvent {
                category: HeartbeatCategory::Important,
                source: format!("notification:{}", item.provider),
                source_event_id: item.id.clone(),
                overlap_key: compute_overlap_key(
                    HeartbeatCategory::Important,
                    &title,
                    item.received_at,
                ),
                fingerprint: stable_key(&format!("notification:{}", item.id)),
                title,
                body,
                deep_link: Some("/notifications".to_string()),
                meeting_url: None,
                anchor_at: item.received_at,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    fn conn(id: &str, toolkit: &str, status: &str) -> ComposioConnection {
        ComposioConnection {
            id: id.to_string(),
            toolkit: toolkit.to_string(),
            status: status.to_string(),
            created_at: None,
            account_email: None,
            workspace: None,
            username: None,
        }
    }

    #[test]
    fn calendar_selection_rotates_across_tick_buckets() {
        let connections = vec![
            conn("cal-1", "googlecalendar", "ACTIVE"),
            conn("cal-2", "google_calendar", "CONNECTED"),
            conn("cal-3", "calendar", "ACTIVE"),
        ];
        let first_tick = Utc.timestamp_opt(0, 0).single().unwrap();
        let second_tick = Utc.timestamp_opt(300, 0).single().unwrap();

        let first = select_calendar_connections_for_tick(connections.clone(), 2, first_tick, 5)
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();
        let second = select_calendar_connections_for_tick(connections, 2, second_tick, 5)
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();

        assert_eq!(first, vec!["cal-1", "cal-2"]);
        assert_eq!(second, vec!["cal-2", "cal-3"]);
    }

    #[test]
    fn calendar_selection_uses_heartbeat_interval_floor() {
        let connections = vec![
            conn("cal-1", "googlecalendar", "ACTIVE"),
            conn("cal-2", "google_calendar", "CONNECTED"),
            conn("cal-3", "calendar", "ACTIVE"),
        ];
        let one_minute_later = Utc.timestamp_opt(60, 0).single().unwrap();
        let five_minutes_later = Utc.timestamp_opt(300, 0).single().unwrap();

        let first =
            select_calendar_connections_for_tick(connections.clone(), 2, one_minute_later, 1)
                .into_iter()
                .map(|c| c.id)
                .collect::<Vec<_>>();
        let second = select_calendar_connections_for_tick(connections, 2, five_minutes_later, 1)
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();

        assert_eq!(first, vec!["cal-1", "cal-2"]);
        assert_eq!(second, vec!["cal-2", "cal-3"]);
    }

    #[test]
    fn calendar_selection_filters_inactive_and_non_calendar_connections() {
        let connections = vec![
            conn("slack", "slack", "ACTIVE"),
            conn("pending-cal", "googlecalendar", "PENDING"),
            conn("active-cal", "googlecalendar", "ACTIVE"),
        ];
        let now = Utc.timestamp_opt(0, 0).single().unwrap();

        let selected = select_calendar_connections_for_tick(connections, 10, now, 5)
            .into_iter()
            .map(|c| c.id)
            .collect::<Vec<_>>();

        assert_eq!(selected, vec!["active-cal"]);
    }

    // ── extract_meeting_url_from_map ─────────────────────────────

    fn map_from_value(v: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        match v {
            serde_json::Value::Object(m) => m,
            _ => panic!("expected object"),
        }
    }

    #[test]
    fn extract_meeting_url_picks_hangout_link() {
        let map = map_from_value(serde_json::json!({
            "hangoutLink": "https://meet.google.com/abc-defg-hij",
            "summary": "Standup"
        }));
        assert_eq!(
            extract_meeting_url_from_map(&map).as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
    }

    #[test]
    fn extract_meeting_url_picks_conference_data_entry_point() {
        let map = map_from_value(serde_json::json!({
            "conferenceData": {
                "entryPoints": [
                    { "entryPointType": "phone", "uri": "tel:+1234567890" },
                    { "entryPointType": "video", "uri": "https://meet.google.com/xyz-uvwx-yz1" }
                ]
            }
        }));
        assert_eq!(
            extract_meeting_url_from_map(&map).as_deref(),
            Some("https://meet.google.com/xyz-uvwx-yz1")
        );
    }

    #[test]
    fn extract_meeting_url_picks_zoom_from_location() {
        let map = map_from_value(serde_json::json!({
            "location": "https://zoom.us/j/123456789"
        }));
        assert_eq!(
            extract_meeting_url_from_map(&map).as_deref(),
            Some("https://zoom.us/j/123456789")
        );
    }

    #[test]
    fn extract_meeting_url_picks_url_out_of_free_form_location() {
        // A label + URL is the common calendar shape; we must return only the
        // parseable URL, not the whole string (which url::Url::parse rejects).
        let map = map_from_value(serde_json::json!({
            "location": "Zoom Meeting: (https://zoom.us/j/123456789), dial-in optional"
        }));
        assert_eq!(
            extract_meeting_url_from_map(&map).as_deref(),
            Some("https://zoom.us/j/123456789")
        );
    }

    #[test]
    fn extract_meeting_url_rejects_unparseable_location() {
        // Mentions a host substring but has no real URL — must not leak a value
        // the join handler would reject.
        let map = map_from_value(serde_json::json!({
            "location": "Conference Room — ask host for the zoom.us link"
        }));
        assert_eq!(extract_meeting_url_from_map(&map), None);
    }

    #[test]
    fn extract_meeting_url_rejects_non_meeting_hangout_link() {
        let map = map_from_value(serde_json::json!({
            "hangoutLink": "https://not-a-meeting-host.example.com/room/abc"
        }));
        assert_eq!(extract_meeting_url_from_map(&map), None);
    }

    #[test]
    fn extract_meeting_url_returns_none_for_plain_event() {
        let map = map_from_value(serde_json::json!({
            "summary": "Lunch",
            "location": "Office kitchen"
        }));
        assert_eq!(extract_meeting_url_from_map(&map), None);
    }

    #[test]
    fn extract_meeting_url_strips_trailing_period() {
        // url::Url::parse accepts a trailing period as a path segment, but it
        // produces a subtly different URL. Strip it at the token-trim level.
        let map = map_from_value(serde_json::json!({
            "location": "Join the call: https://zoom.us/j/999888777."
        }));
        assert_eq!(
            extract_meeting_url_from_map(&map).as_deref(),
            Some("https://zoom.us/j/999888777")
        );
    }
}
