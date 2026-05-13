//! Heartbeat and cron configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Heartbeat configuration — periodic background loop that evaluates
/// HEARTBEAT.md tasks against workspace state using local model inference.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct HeartbeatConfig {
    /// Enable the heartbeat loop.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Tick interval in minutes (minimum 5).
    #[serde(default = "default_interval_minutes")]
    pub interval_minutes: u32,
    /// Enable subconscious inference (local model evaluation).
    /// When false, the heartbeat only counts tasks without reasoning.
    #[serde(default = "default_true")]
    pub inference_enabled: bool,
    /// Maximum token budget for the situation report (default 40k).
    #[serde(default = "default_context_budget")]
    pub context_budget_tokens: u32,
    /// Enable proactive notifications for upcoming meetings.
    #[serde(default = "default_true")]
    pub notify_meetings: bool,
    /// Enable proactive notifications for reminders and scheduled items.
    #[serde(default = "default_true")]
    pub notify_reminders: bool,
    /// Enable proactive notifications for urgent/relevant events.
    #[serde(default = "default_true")]
    pub notify_relevant_events: bool,
    /// Allow heartbeat proactive events to also deliver to active external channel.
    /// Defaults to false and acts as an explicit consent gate.
    #[serde(default)]
    pub external_delivery_enabled: bool,
    /// Maximum lookahead window for meeting notifications.
    #[serde(default = "default_meeting_lookahead_minutes")]
    pub meeting_lookahead_minutes: u32,
    /// Maximum lookahead window for reminder notifications.
    #[serde(default = "default_reminder_lookahead_minutes")]
    pub reminder_lookahead_minutes: u32,
}

fn default_context_budget() -> u32 {
    40_000
}

fn default_true() -> bool {
    true
}

fn default_interval_minutes() -> u32 {
    5
}

fn default_meeting_lookahead_minutes() -> u32 {
    120
}

fn default_reminder_lookahead_minutes() -> u32 {
    30
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            interval_minutes: default_interval_minutes(),
            inference_enabled: default_true(),
            context_budget_tokens: default_context_budget(),
            notify_meetings: default_true(),
            notify_reminders: default_true(),
            notify_relevant_events: default_true(),
            external_delivery_enabled: false,
            meeting_lookahead_minutes: default_meeting_lookahead_minutes(),
            reminder_lookahead_minutes: default_reminder_lookahead_minutes(),
        }
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
