//! Heartbeat and cron configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Heartbeat configuration — periodic background loop that evaluates
/// HEARTBEAT.md tasks and proactive notification sources.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HeartbeatConfig {
    /// Enable the heartbeat loop. Opt-in because ticks may call hosted models
    /// and integration APIs depending on routing and enabled collectors.
    #[serde(default)]
    pub enabled: bool,
    /// Tick interval in minutes (minimum 5).
    #[serde(default = "default_interval_minutes")]
    pub interval_minutes: u32,
    /// Enable subconscious inference. When false, heartbeat only counts tasks
    /// without model-backed reasoning.
    #[serde(default)]
    pub inference_enabled: bool,
    /// Maximum token budget for the situation report (default 40k).
    #[serde(default = "default_context_budget")]
    pub context_budget_tokens: u32,
    /// Enable proactive notifications for upcoming meetings.
    #[serde(default)]
    pub notify_meetings: bool,
    /// Enable proactive notifications for reminders and scheduled items.
    #[serde(default)]
    pub notify_reminders: bool,
    /// Enable proactive notifications for urgent/relevant events.
    #[serde(default)]
    pub notify_relevant_events: bool,
    /// Allow heartbeat proactive events to also deliver to active external channel.
    /// Defaults to false and acts as an explicit consent gate.
    #[serde(default)]
    pub external_delivery_enabled: bool,
    /// Maximum lookahead window for meeting notifications.
    #[serde(default = "default_meeting_lookahead_minutes")]
    pub meeting_lookahead_minutes: u32,
    /// Maximum active calendar connections polled per heartbeat planner tick.
    #[serde(
        default = "default_max_calendar_connections_per_tick",
        deserialize_with = "deserialize_calendar_connection_limit"
    )]
    pub max_calendar_connections_per_tick: u32,
    /// Maximum lookahead window for reminder notifications.
    #[serde(default = "default_reminder_lookahead_minutes")]
    pub reminder_lookahead_minutes: u32,
}

fn default_context_budget() -> u32 {
    40_000
}

fn default_interval_minutes() -> u32 {
    5
}

fn default_meeting_lookahead_minutes() -> u32 {
    120
}

fn default_max_calendar_connections_per_tick() -> u32 {
    2
}

fn deserialize_calendar_connection_limit<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<u32>::deserialize(deserializer)?;
    Ok(value
        .unwrap_or_else(default_max_calendar_connections_per_tick)
        .max(1))
}

fn default_reminder_lookahead_minutes() -> u32 {
    30
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_minutes: default_interval_minutes(),
            inference_enabled: false,
            context_budget_tokens: default_context_budget(),
            notify_meetings: false,
            notify_reminders: false,
            notify_relevant_events: false,
            external_delivery_enabled: false,
            meeting_lookahead_minutes: default_meeting_lookahead_minutes(),
            max_calendar_connections_per_tick: default_max_calendar_connections_per_tick(),
            reminder_lookahead_minutes: default_reminder_lookahead_minutes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heartbeat_defaults_are_opt_in() {
        let config = HeartbeatConfig::default();
        assert!(!config.enabled);
        assert!(!config.inference_enabled);
        assert!(!config.notify_meetings);
        assert!(!config.notify_reminders);
        assert!(!config.notify_relevant_events);
        assert!(!config.external_delivery_enabled);
        assert_eq!(config.interval_minutes, 5);
        assert_eq!(config.max_calendar_connections_per_tick, 2);
    }

    #[test]
    fn heartbeat_deserialization_fills_opt_in_defaults() {
        let config: HeartbeatConfig = serde_json::from_str("{}").unwrap();
        assert!(!config.enabled);
        assert!(!config.inference_enabled);
        assert!(!config.notify_meetings);
        assert!(!config.notify_reminders);
        assert!(!config.notify_relevant_events);
        assert!(!config.external_delivery_enabled);
        assert_eq!(config.interval_minutes, 5);
        assert_eq!(config.max_calendar_connections_per_tick, 2);
        assert_eq!(config.meeting_lookahead_minutes, 120);
        assert_eq!(config.reminder_lookahead_minutes, 30);

        let partial: HeartbeatConfig =
            serde_json::from_str(r#"{"enabled":true,"interval_minutes":15}"#).unwrap();
        assert!(partial.enabled);
        assert_eq!(partial.interval_minutes, 15);
        assert!(!partial.inference_enabled);
        assert!(!partial.notify_meetings);
        assert_eq!(partial.max_calendar_connections_per_tick, 2);

        let zero_cap: HeartbeatConfig =
            serde_json::from_str(r#"{"max_calendar_connections_per_tick":0}"#).unwrap();
        assert_eq!(zero_cap.max_calendar_connections_per_tick, 1);

        let null_cap: HeartbeatConfig =
            serde_json::from_str(r#"{"max_calendar_connections_per_tick":null}"#).unwrap();
        assert_eq!(null_cap.max_calendar_connections_per_tick, 2);

        let explicit_cap: HeartbeatConfig =
            serde_json::from_str(r#"{"max_calendar_connections_per_tick":4}"#).unwrap();
        assert_eq!(explicit_cap.max_calendar_connections_per_tick, 4);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CronConfig {
    #[serde(default = "default_cron_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cron_max_run_history")]
    pub max_run_history: usize,
}

fn default_cron_enabled() -> bool {
    true
}

fn default_cron_max_run_history() -> usize {
    50
}

impl Default for CronConfig {
    fn default() -> Self {
        Self {
            enabled: default_cron_enabled(),
            max_run_history: default_cron_max_run_history(),
        }
    }
}
