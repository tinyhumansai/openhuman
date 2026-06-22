use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use futures::stream::StreamExt;
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

/// Bound on how many calendar connections are polled concurrently per tick.
///
/// `select_calendar_connections_for_tick` already caps the selected set to
/// `max_calendar_connections_per_tick` (default 2), so this only matters when a
/// user raises that cap. Kept small to avoid opening more than a handful of
/// sockets against Composio/Google Calendar in a single heartbeat tick.
const CALENDAR_FANOUT_CONCURRENCY: usize = 4;

/// Narrow seam over the mode-aware Composio client so the calendar fan-out can
/// be unit-tested with a fake executor (no network). Both real variants
/// (`Backend` / `Direct`) and the fake forward through the identical
/// arg-building + extraction path.
#[async_trait]
trait CalendarExecutor {
    /// Stable label for the underlying client variant, surfaced on the failure
    /// log so backend vs direct dispatch stays distinguishable post-refactor.
    fn kind_label(&self) -> &'static str;

    /// Execute one `GOOGLECALENDAR_EVENTS_LIST` call for a single connection.
    async fn execute(
        &self,
        slug: &str,
        arguments: Option<serde_json::Value>,
    ) -> anyhow::Result<ComposioExecuteResponse>;
}

/// Real executor wrapping the resolved mode-aware client. Holds borrows only —
/// constructed per tick, dropped when the fan-out completes.
struct ComposioCalendarExecutor<'a> {
    kind: &'a ComposioClientKind,
    entity_id: &'a str,
}

#[async_trait]
impl CalendarExecutor for ComposioCalendarExecutor<'_> {
    fn kind_label(&self) -> &'static str {
        match self.kind {
            ComposioClientKind::Backend(_) => "backend",
            ComposioClientKind::Direct(_) => "direct",
        }
    }

    async fn execute(
        &self,
        slug: &str,
        arguments: Option<serde_json::Value>,
    ) -> anyhow::Result<ComposioExecuteResponse> {
        match self.kind {
            ComposioClientKind::Backend(client) => client.execute_tool(slug, arguments).await,
            ComposioClientKind::Direct(direct) => {
                direct_execute(direct, slug, arguments, self.entity_id, None).await
            }
        }
    }
}

/// Fetch + extract upcoming meetings for one calendar connection.
///
/// A failed fetch contributes no events (returns an empty `Vec`), matching the
/// serial loop's `continue` so one broken connection never poisons the tick.
async fn fetch_calendar_events_for_connection(
    executor: &(dyn CalendarExecutor + Sync),
    conn: &ComposioConnection,
    meeting_lookahead_minutes: u32,
    now: DateTime<Utc>,
    end_window: DateTime<Utc>,
) -> Vec<PendingEvent> {
    let toolkit = conn.normalized_toolkit();

    // Build base args, then let the shared transformer fill in `timeZone` +
    // `singleEvents` so this poller behaves identically to the agent-driven
    // dispatcher path (issue #1714). Routing both call sites through the same
    // helper means a future change to the defaulting policy only has to land in
    // one place.
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
        lookahead_minutes = meeting_lookahead_minutes,
        "[composio][heartbeat-planner] applying calendar query defaults pre-poll"
    );
    let arguments = crate::openhuman::composio::googlecalendar_args::apply_calendar_query_defaults(
        "GOOGLECALENDAR_EVENTS_LIST",
        Some(arguments),
        &iana,
    );

    match executor
        .execute("GOOGLECALENDAR_EVENTS_LIST", arguments)
        .await
    {
        // Composio encodes provider-side failures in `successful`/`error` while
        // still returning `Ok(_)` (the repo-wide `if !resp.successful` convention),
        // so an unsuccessful list must take the same warn + empty path as a
        // transport `Err` — otherwise an error payload falls through to
        // `extract_calendar_events`, losing the diagnostic and risking stale data.
        Ok(resp) if resp.successful => {
            extract_calendar_events(&resp.data, &toolkit, &conn.id, now, end_window)
        }
        Ok(resp) => {
            tracing::warn!(
                target: "composio",
                toolkit = %toolkit,
                connection_id = %conn.id,
                kind = executor.kind_label(),
                error = %resp
                    .error
                    .as_deref()
                    .unwrap_or("calendar execute returned unsuccessful=false"),
                "[heartbeat:planner] GOOGLECALENDAR_EVENTS_LIST failed"
            );
            Vec::new()
        }
        Err(error) => {
            tracing::warn!(
                target: "composio",
                toolkit = %toolkit,
                connection_id = %conn.id,
                kind = executor.kind_label(),
                error = %error,
                "[heartbeat:planner] GOOGLECALENDAR_EVENTS_LIST failed"
            );
            Vec::new()
        }
    }
}

/// Drive the per-connection fetches with bounded concurrency.
///
/// `buffered(K)` yields results in input order, so the flattened event stream is
/// identical to the old strictly-serial loop; only the wall-clock latency drops
/// (from the sum of all fetches to roughly `ceil(N / K)` round-trips). Single
/// connections (the common case) cost nothing extra.
async fn collect_calendar_events_buffered(
    executor: &(dyn CalendarExecutor + Sync),
    connections: &[ComposioConnection],
    meeting_lookahead_minutes: u32,
    now: DateTime<Utc>,
    end_window: DateTime<Utc>,
) -> Vec<PendingEvent> {
    // Materialize the per-connection futures into a `Vec` before `stream::iter`
    // — handing it a lazy `Map` adaptor trips the "implementation of `Send` is
    // not general enough" HRTB error.
    let fetch_futs: Vec<_> = connections
        .iter()
        .map(|conn| {
            fetch_calendar_events_for_connection(
                executor,
                conn,
                meeting_lookahead_minutes,
                now,
                end_window,
            )
        })
        .collect();

    futures::stream::iter(fetch_futs)
        .buffered(CALENDAR_FANOUT_CONCURRENCY)
        .collect::<Vec<Vec<PendingEvent>>>()
        .await
        .into_iter()
        .flatten()
        .collect()
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

    let executor = ComposioCalendarExecutor {
        kind: &kind,
        entity_id: &config.composio.entity_id,
    };
    collect_calendar_meetings_with(
        &executor,
        connections,
        now,
        config.heartbeat.meeting_lookahead_minutes.max(1),
        config.heartbeat.max_calendar_connections_per_tick.max(1) as usize,
        config.heartbeat.interval_minutes,
    )
    .await
}

/// Tick-rotation selection + bounded fan-out, decoupled from the concrete
/// Composio client so it is unit-testable with a fake `CalendarExecutor`.
///
/// `collect_calendar_meetings` only builds the real executor and delegates here;
/// the rotation cap (`select_calendar_connections_for_tick`) and lookahead window
/// run exactly as before.
async fn collect_calendar_meetings_with(
    executor: &(dyn CalendarExecutor + Sync),
    connections: Vec<ComposioConnection>,
    now: DateTime<Utc>,
    meeting_lookahead_minutes: u32,
    calendar_connection_limit: usize,
    interval_minutes: u32,
) -> Vec<PendingEvent> {
    let end_window = now + Duration::minutes(i64::from(meeting_lookahead_minutes));
    let selected = select_calendar_connections_for_tick(
        connections,
        calendar_connection_limit,
        now,
        interval_minutes,
    );
    tracing::debug!(
        executor = executor.kind_label(),
        selected = selected.len(),
        calendar_connection_limit,
        meeting_lookahead_minutes,
        interval_minutes,
        concurrency = CALENDAR_FANOUT_CONCURRENCY,
        now = %now,
        end_window = %end_window,
        "[heartbeat:planner] calendar fan-out start"
    );
    let events = collect_calendar_events_buffered(
        executor,
        &selected,
        meeting_lookahead_minutes,
        now,
        end_window,
    )
    .await;
    tracing::debug!(
        executor = executor.kind_label(),
        selected = selected.len(),
        events = events.len(),
        "[heartbeat:planner] calendar fan-out complete"
    );
    events
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

    // ── Calendar fan-out (bounded concurrency) ───────────────────────────

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Build a Composio `data` payload carrying a single in-window event whose
    /// summary becomes the extracted `PendingEvent` title.
    fn in_window_event(summary: &str, start: DateTime<Utc>) -> serde_json::Value {
        json!({
            "items": [
                {
                    "id": summary,
                    "summary": summary,
                    "start": { "dateTime": start.to_rfc3339() }
                }
            ]
        })
    }

    /// In-memory `CalendarExecutor` keyed by `connectionId`, with an optional
    /// per-call delay and an observed-max-concurrency probe.
    struct FakeExecutor {
        responses: std::collections::HashMap<String, Result<serde_json::Value, String>>,
        /// Connections whose execute returns `Ok(successful=false)` carrying the
        /// given would-be-event payload — models a provider-side failure that
        /// still arrives as `Ok(_)`.
        unsuccessful: std::collections::HashMap<String, serde_json::Value>,
        inflight: Arc<AtomicUsize>,
        max_inflight: Arc<AtomicUsize>,
        delay_ms: u64,
    }

    impl FakeExecutor {
        fn new(delay_ms: u64) -> Self {
            Self {
                responses: std::collections::HashMap::new(),
                unsuccessful: std::collections::HashMap::new(),
                inflight: Arc::new(AtomicUsize::new(0)),
                max_inflight: Arc::new(AtomicUsize::new(0)),
                delay_ms,
            }
        }

        fn with(mut self, conn_id: &str, resp: Result<serde_json::Value, String>) -> Self {
            self.responses.insert(conn_id.to_string(), resp);
            self
        }

        /// Register a connection that returns `Ok` but with `successful=false`,
        /// carrying `data` that *would* parse to an event if it were extracted.
        fn with_unsuccessful(mut self, conn_id: &str, data: serde_json::Value) -> Self {
            self.unsuccessful.insert(conn_id.to_string(), data);
            self
        }
    }

    #[async_trait]
    impl CalendarExecutor for FakeExecutor {
        fn kind_label(&self) -> &'static str {
            "fake"
        }

        async fn execute(
            &self,
            _slug: &str,
            arguments: Option<serde_json::Value>,
        ) -> anyhow::Result<ComposioExecuteResponse> {
            let current = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.max_inflight.fetch_max(current, Ordering::SeqCst);
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            self.inflight.fetch_sub(1, Ordering::SeqCst);

            let conn_id = arguments
                .as_ref()
                .and_then(|a| a.get("connectionId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(data) = self.unsuccessful.get(&conn_id) {
                return Ok(ComposioExecuteResponse {
                    data: data.clone(),
                    successful: false,
                    error: Some("provider rejected the request".to_string()),
                    cost_usd: 0.0,
                    markdown_formatted: None,
                });
            }
            match self.responses.get(&conn_id) {
                Some(Ok(data)) => Ok(ComposioExecuteResponse {
                    data: data.clone(),
                    successful: true,
                    error: None,
                    cost_usd: 0.0,
                    markdown_formatted: None,
                }),
                Some(Err(msg)) => Err(anyhow::anyhow!(msg.clone())),
                None => Err(anyhow::anyhow!("no canned response for {conn_id}")),
            }
        }
    }

    #[tokio::test]
    async fn calendar_fanout_preserves_connection_order() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let end_window = now + Duration::minutes(120);
        let start = now + Duration::minutes(30);

        // 5 connections > CALENDAR_FANOUT_CONCURRENCY (4), so the window wraps.
        let conns: Vec<ComposioConnection> = (1..=5)
            .map(|n| conn(&format!("cal-{n}"), "googlecalendar", "ACTIVE"))
            .collect();

        let mut exec = FakeExecutor::new(0);
        for n in 1..=5 {
            exec = exec.with(
                &format!("cal-{n}"),
                Ok(in_window_event(&format!("evt-{n}"), start)),
            );
        }

        let events = collect_calendar_events_buffered(&exec, &conns, 120, now, end_window).await;
        let titles: Vec<String> = events.into_iter().map(|e| e.title).collect();
        assert_eq!(
            titles,
            vec!["evt-1", "evt-2", "evt-3", "evt-4", "evt-5"],
            "buffered fan-out must preserve per-connection input order"
        );
    }

    #[tokio::test]
    async fn calendar_fanout_runs_connections_concurrently() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let end_window = now + Duration::minutes(120);
        let start = now + Duration::minutes(30);

        // Drive *more* connections than the cap (N = CALENDAR_FANOUT_CONCURRENCY
        // + 2) so this test fails if the implementation ever regresses to an
        // unbounded fan-out: with exactly `CALENDAR_FANOUT_CONCURRENCY`
        // connections the cap is indistinguishable from no cap at all.
        let total = CALENDAR_FANOUT_CONCURRENCY + 2;
        let conns: Vec<ComposioConnection> = (1..=total)
            .map(|n| conn(&format!("cal-{n}"), "googlecalendar", "ACTIVE"))
            .collect();

        let mut exec = FakeExecutor::new(30);
        for n in 1..=total {
            exec = exec.with(
                &format!("cal-{n}"),
                Ok(in_window_event(&format!("evt-{n}"), start)),
            );
        }
        let max_inflight = exec.max_inflight.clone();

        let events = collect_calendar_events_buffered(&exec, &conns, 120, now, end_window).await;
        assert_eq!(events.len(), total);
        let observed = max_inflight.load(Ordering::SeqCst);
        assert!(
            observed > 1,
            "fan-out must overlap fetches (observed max in-flight = {observed})"
        );
        assert!(
            observed <= CALENDAR_FANOUT_CONCURRENCY,
            "fan-out must stay within the concurrency cap of {CALENDAR_FANOUT_CONCURRENCY} \
             (observed max in-flight = {observed})"
        );
    }

    #[tokio::test]
    async fn calendar_fanout_isolates_failed_connection() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let end_window = now + Duration::minutes(120);
        let start = now + Duration::minutes(30);

        let conns = vec![
            conn("cal-1", "googlecalendar", "ACTIVE"),
            conn("cal-2", "googlecalendar", "ACTIVE"),
            conn("cal-3", "googlecalendar", "ACTIVE"),
        ];

        // Middle connection errors — it must drop out without poisoning the
        // surviving two, which keep their input order.
        let exec = FakeExecutor::new(0)
            .with("cal-1", Ok(in_window_event("evt-1", start)))
            .with("cal-2", Err("boom".to_string()))
            .with("cal-3", Ok(in_window_event("evt-3", start)));

        let events = collect_calendar_events_buffered(&exec, &conns, 120, now, end_window).await;
        let titles: Vec<String> = events.into_iter().map(|e| e.title).collect();
        assert_eq!(
            titles,
            vec!["evt-1", "evt-3"],
            "a failed connection contributes no events while order is preserved"
        );
    }

    #[tokio::test]
    async fn calendar_fanout_drops_unsuccessful_response() {
        let now = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let end_window = now + Duration::minutes(120);
        let start = now + Duration::minutes(30);

        let conns = vec![
            conn("cal-1", "googlecalendar", "ACTIVE"),
            conn("cal-2", "googlecalendar", "ACTIVE"),
        ];

        // cal-2 returns Ok(successful=false) carrying a payload that *would*
        // parse to an event — proving the unsuccessful path warns + drops it
        // instead of falling through to `extract_calendar_events`.
        let exec = FakeExecutor::new(0)
            .with("cal-1", Ok(in_window_event("evt-1", start)))
            .with_unsuccessful("cal-2", in_window_event("evt-2-must-not-appear", start));

        let events = collect_calendar_events_buffered(&exec, &conns, 120, now, end_window).await;
        let titles: Vec<String> = events.into_iter().map(|e| e.title).collect();
        assert_eq!(
            titles,
            vec!["evt-1"],
            "an Ok(successful=false) response must contribute no events, not its error payload"
        );
    }

    #[tokio::test]
    async fn collect_calendar_meetings_with_selects_and_fans_out() {
        // ts=1_700_000_000 with interval=5 → tick_index=5_666_666, which is even,
        // so offset = tick_index % 2 = 0: the two connections are selected in
        // input order (identity rotation), and the fan-out preserves that order.
        let now = Utc.timestamp_opt(1_700_000_000, 0).single().unwrap();
        let start = now + Duration::minutes(30);

        let conns = vec![
            conn("cal-1", "googlecalendar", "ACTIVE"),
            conn("cal-2", "googlecalendar", "ACTIVE"),
        ];

        let exec = FakeExecutor::new(0)
            .with("cal-1", Ok(in_window_event("evt-1", start)))
            .with("cal-2", Ok(in_window_event("evt-2", start)));

        // limit=2 >= N so selection keeps both; interval=5.
        let events = collect_calendar_meetings_with(&exec, conns, now, 120, 2, 5).await;
        let titles: Vec<String> = events.into_iter().map(|e| e.title).collect();
        assert_eq!(
            titles,
            vec!["evt-1", "evt-2"],
            "wrapper must select the tick's connections and fan-out in order"
        );
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
