//! Heartbeat, cron, and subconscious mode configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Subconscious operating mode — controls tool access and tick frequency.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SubconsciousMode {
    /// Disabled — the subconscious loop does not run.
    #[default]
    Off,
    /// Read-only observation every 30 minutes. Memory recall and file
    /// reading only — no writes, no sub-agent spawning.
    Simple,
    /// Full tool access every 5 minutes. Can write, spawn sub-agents,
    /// and delegate tasks to the orchestrator.
    Aggressive,
    /// Event-driven: full tool access, woken by the trigger pipeline (cron /
    /// user message / Composio webhook / sub-agent conclusion) rather than a
    /// fixed interval. The cron cadence is the periodic-heartbeat fallback.
    #[serde(rename = "event_driven")]
    EventDriven,
}

impl SubconsciousMode {
    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }

    pub fn default_interval_minutes(self) -> u32 {
        match self {
            Self::Off => 5,
            Self::Simple => 30,
            Self::Aggressive => 5,
            // Periodic heartbeat fallback cadence; real reactivity comes
            // from the event pipeline, not the timer.
            Self::EventDriven => 5,
        }
    }

    pub fn is_read_only(self) -> bool {
        matches!(self, Self::Simple)
    }

    /// Whether this mode runs the event-driven trigger pipeline.
    pub fn is_event_driven(self) -> bool {
        matches!(self, Self::EventDriven)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Simple => "simple",
            Self::Aggressive => "aggressive",
            Self::EventDriven => "event_driven",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "simple" => Self::Simple,
            "aggressive" => Self::Aggressive,
            "event_driven" => Self::EventDriven,
            _ => Self::Off,
        }
    }
}

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
    /// Subconscious operating mode. Controls tool access and tick frequency.
    /// Off (default) = disabled. Simple = read-only every 30 min.
    /// Aggressive = full access every 5 min.
    #[serde(default)]
    pub subconscious_mode: SubconsciousMode,
    /// Enable the event-driven subconscious trigger pipeline (cron / user
    /// message / Composio webhook / sub-agent conclusion → LLM gate →
    /// long-lived orchestrator session). Opt-in: when false, the legacy
    /// interval-only heartbeat path is unchanged.
    #[serde(default)]
    pub triggers_enabled: bool,
    /// Per-hour cap on trigger promotions (long-lived session runs). Bounds
    /// the always-on loop's spend ceiling.
    #[serde(default = "default_max_promotions_per_hour")]
    pub max_promotions_per_hour: u32,
}

fn default_max_promotions_per_hour() -> u32 {
    30
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
            subconscious_mode: SubconsciousMode::Off,
            triggers_enabled: false,
            max_promotions_per_hour: default_max_promotions_per_hour(),
        }
    }
}

impl HeartbeatConfig {
    /// Resolve the effective subconscious mode, handling backward
    /// compatibility for configs that pre-date the `subconscious_mode`
    /// field. If `subconscious_mode` is explicitly set (not Off-by-default),
    /// use it. Otherwise, if the legacy `enabled && inference_enabled`
    /// flags are true, treat as `Simple`.
    pub fn effective_subconscious_mode(&self) -> SubconsciousMode {
        if self.subconscious_mode != SubconsciousMode::Off {
            return self.subconscious_mode;
        }
        if self.enabled && self.inference_enabled {
            SubconsciousMode::Simple
        } else {
            SubconsciousMode::Off
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
        assert_eq!(config.subconscious_mode, SubconsciousMode::Off);
        // Event-driven trigger pipeline is opt-in (back-compat: old configs
        // without these keys deserialize to the legacy interval-only path).
        assert!(!config.triggers_enabled);
        assert_eq!(config.max_promotions_per_hour, 30);
    }

    #[test]
    fn legacy_config_without_trigger_keys_deserializes() {
        // A config file predating the trigger pipeline must still parse and
        // default to the disabled (legacy) path.
        let legacy = r#"{ "enabled": true, "inference_enabled": true }"#;
        let config: HeartbeatConfig = serde_json::from_str(legacy).unwrap();
        assert!(!config.triggers_enabled);
        assert_eq!(config.max_promotions_per_hour, 30);
        assert_eq!(
            config.effective_subconscious_mode(),
            SubconsciousMode::Simple
        );
    }

    #[test]
    fn event_driven_mode_serde_and_helpers() {
        assert_eq!(
            serde_json::to_string(&SubconsciousMode::EventDriven).unwrap(),
            r#""event_driven""#
        );
        assert_eq!(
            serde_json::from_str::<SubconsciousMode>(r#""event_driven""#).unwrap(),
            SubconsciousMode::EventDriven
        );
        assert!(SubconsciousMode::EventDriven.is_enabled());
        assert!(SubconsciousMode::EventDriven.is_event_driven());
        assert!(!SubconsciousMode::Aggressive.is_event_driven());
        assert!(!SubconsciousMode::EventDriven.is_read_only());
        assert_eq!(
            SubconsciousMode::from_str_lossy("event_driven"),
            SubconsciousMode::EventDriven
        );
    }

    #[test]
    fn subconscious_mode_serde_round_trip() {
        assert_eq!(
            serde_json::to_string(&SubconsciousMode::Simple).unwrap(),
            r#""simple""#
        );
        assert_eq!(
            serde_json::from_str::<SubconsciousMode>(r#""aggressive""#).unwrap(),
            SubconsciousMode::Aggressive
        );
        assert_eq!(SubconsciousMode::default(), SubconsciousMode::Off);
    }

    #[test]
    fn subconscious_mode_helpers() {
        assert!(!SubconsciousMode::Off.is_enabled());
        assert!(SubconsciousMode::Simple.is_enabled());
        assert!(SubconsciousMode::Aggressive.is_enabled());
        assert!(SubconsciousMode::Simple.is_read_only());
        assert!(!SubconsciousMode::Aggressive.is_read_only());
        assert_eq!(SubconsciousMode::Simple.default_interval_minutes(), 30);
        assert_eq!(SubconsciousMode::Aggressive.default_interval_minutes(), 5);
    }

    #[test]
    fn effective_mode_backward_compat() {
        let mut config = HeartbeatConfig::default();
        assert_eq!(config.effective_subconscious_mode(), SubconsciousMode::Off);

        config.enabled = true;
        config.inference_enabled = true;
        assert_eq!(
            config.effective_subconscious_mode(),
            SubconsciousMode::Simple
        );

        config.subconscious_mode = SubconsciousMode::Aggressive;
        assert_eq!(
            config.effective_subconscious_mode(),
            SubconsciousMode::Aggressive
        );
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
