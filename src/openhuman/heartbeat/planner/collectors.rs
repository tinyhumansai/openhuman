use chrono::{DateTime, Duration, Utc};
use serde_json::json;

use crate::openhuman::composio::client::{
    create_composio_client, direct_execute, direct_list_connections, ComposioClientKind,
};
use crate::openhuman::composio::types::ComposioExecuteResponse;
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
    for conn in connections.into_iter().filter(|c| c.is_active()) {
        let toolkit = conn.normalized_toolkit();
        if toolkit != "googlecalendar" && toolkit != "google_calendar" && toolkit != "calendar" {
            continue;
        }

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
                anchor_at: item.received_at,
            }
        })
        .collect()
}
