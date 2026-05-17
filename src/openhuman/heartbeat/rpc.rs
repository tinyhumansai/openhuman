use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::{debug, warn};

use crate::openhuman::config::{self, Config};
use crate::rpc::RpcOutcome;

use super::planner;

#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatSettingsPatch {
    pub enabled: Option<bool>,
    pub interval_minutes: Option<u32>,
    pub inference_enabled: Option<bool>,
    pub notify_meetings: Option<bool>,
    pub notify_reminders: Option<bool>,
    pub notify_relevant_events: Option<bool>,
    pub external_delivery_enabled: Option<bool>,
    pub meeting_lookahead_minutes: Option<u32>,
    pub max_calendar_connections_per_tick: Option<u32>,
    pub reminder_lookahead_minutes: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HeartbeatSettingsView {
    pub enabled: bool,
    pub interval_minutes: u32,
    pub inference_enabled: bool,
    pub notify_meetings: bool,
    pub notify_reminders: bool,
    pub notify_relevant_events: bool,
    pub external_delivery_enabled: bool,
    pub meeting_lookahead_minutes: u32,
    pub max_calendar_connections_per_tick: u32,
    pub reminder_lookahead_minutes: u32,
}

pub async fn settings_get() -> Result<RpcOutcome<serde_json::Value>, String> {
    debug!("[heartbeat][rpc] settings_get: entry");
    let config = config::rpc::load_config_with_timeout().await.map_err(|e| {
        warn!("[heartbeat][rpc] settings_get: load_config failed: {e}");
        e
    })?;
    debug!("[heartbeat][rpc] settings_get: exit ok");
    Ok(RpcOutcome::single_log(
        json!({ "settings": view(&config) }),
        "heartbeat settings loaded",
    ))
}

pub async fn settings_set(
    patch: HeartbeatSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    debug!("[heartbeat][rpc] settings_set: entry");
    let mut config = config::rpc::load_config_with_timeout().await.map_err(|e| {
        warn!("[heartbeat][rpc] settings_set: load_config failed: {e}");
        e
    })?;

    if let Some(enabled) = patch.enabled {
        config.heartbeat.enabled = enabled;
    }
    if let Some(interval_minutes) = patch.interval_minutes {
        // Clamp to the 5-minute minimum that HeartbeatEngine::run enforces at runtime.
        config.heartbeat.interval_minutes = interval_minutes.max(5);
    }
    if let Some(inference_enabled) = patch.inference_enabled {
        config.heartbeat.inference_enabled = inference_enabled;
    }
    if let Some(notify_meetings) = patch.notify_meetings {
        config.heartbeat.notify_meetings = notify_meetings;
    }
    if let Some(notify_reminders) = patch.notify_reminders {
        config.heartbeat.notify_reminders = notify_reminders;
    }
    if let Some(notify_relevant_events) = patch.notify_relevant_events {
        config.heartbeat.notify_relevant_events = notify_relevant_events;
    }
    if let Some(external_delivery_enabled) = patch.external_delivery_enabled {
        config.heartbeat.external_delivery_enabled = external_delivery_enabled;
    }
    if let Some(meeting_lookahead_minutes) = patch.meeting_lookahead_minutes {
        config.heartbeat.meeting_lookahead_minutes = meeting_lookahead_minutes.max(1);
    }
    if let Some(max_calendar_connections_per_tick) = patch.max_calendar_connections_per_tick {
        config.heartbeat.max_calendar_connections_per_tick =
            max_calendar_connections_per_tick.max(1);
    }
    if let Some(reminder_lookahead_minutes) = patch.reminder_lookahead_minutes {
        config.heartbeat.reminder_lookahead_minutes = reminder_lookahead_minutes.max(1);
    }

    config.save().await.map_err(|e| {
        warn!("[heartbeat][rpc] settings_set: config.save failed: {e}");
        e.to_string()
    })?;

    if config.heartbeat.enabled {
        debug!("[heartbeat][rpc] settings_set: enabling — running bootstrap_after_login()");
        if let Err(error) = crate::openhuman::subconscious::global::bootstrap_after_login().await {
            warn!("[heartbeat][rpc] settings_set: heartbeat bootstrap failed: {error}");
            return Err(format!(
                "heartbeat settings saved, but failed to start heartbeat loop: {error}"
            ));
        }
    } else {
        debug!("[heartbeat][rpc] settings_set: disabling — stopping heartbeat loop");
        crate::openhuman::subconscious::global::stop_heartbeat_loop().await;
    }

    debug!("[heartbeat][rpc] settings_set: exit ok");
    Ok(RpcOutcome::single_log(
        json!({ "settings": view(&config) }),
        "heartbeat settings saved",
    ))
}

pub async fn tick_now() -> Result<RpcOutcome<serde_json::Value>, String> {
    debug!("[heartbeat][rpc] tick_now: entry");
    let config = config::rpc::load_config_with_timeout().await.map_err(|e| {
        warn!("[heartbeat][rpc] tick_now: load_config failed: {e}");
        e
    })?;
    let summary = planner::evaluate_and_dispatch(&config, Utc::now()).await;
    debug!(
        source_events = summary.source_events,
        deliveries_sent = summary.deliveries_sent,
        "[heartbeat][rpc] tick_now: exit ok"
    );
    Ok(RpcOutcome::single_log(
        json!({ "summary": summary }),
        "heartbeat planner tick completed",
    ))
}

fn view(config: &Config) -> HeartbeatSettingsView {
    HeartbeatSettingsView {
        enabled: config.heartbeat.enabled,
        interval_minutes: config.heartbeat.interval_minutes,
        inference_enabled: config.heartbeat.inference_enabled,
        notify_meetings: config.heartbeat.notify_meetings,
        notify_reminders: config.heartbeat.notify_reminders,
        notify_relevant_events: config.heartbeat.notify_relevant_events,
        external_delivery_enabled: config.heartbeat.external_delivery_enabled,
        meeting_lookahead_minutes: config.heartbeat.meeting_lookahead_minutes,
        max_calendar_connections_per_tick: config.heartbeat.max_calendar_connections_per_tick,
        reminder_lookahead_minutes: config.heartbeat.reminder_lookahead_minutes,
    }
}
