//! Agent activity level — controls how proactive background AI is.
//!
//! Maps a single 0–4 knob into scheduler-gate mode, periodic sync
//! cadence, heartbeat/subconscious toggles, and token budgets.

use schemars::JsonSchema;
use serde_repr::{Deserialize_repr, Serialize_repr};

/// User-facing activity level for background AI work.
///
/// Each level is an opinionated preset that controls multiple subsystems:
/// - Scheduler-gate mode (off / auto / always_on)
/// - Periodic sync cadence (never / daily / hourly / 10min / realtime)
/// - Heartbeat & subconscious inference (disabled / enabled)
/// - Token budget per background cycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize_repr, Deserialize_repr, JsonSchema)]
#[repr(u8)]
pub enum AgentActivityLevel {
    /// No background processing. Syncs only on manual press.
    Off = 0,
    /// Sync sources once per day. No proactive messages.
    Minimal = 1,
    /// Sync every hour. Daily digest. Suggests actions. (default)
    Moderate = 2,
    /// Sync every 10 min. Monitors channels, triages, drafts replies.
    Active = 3,
    /// Real-time sync. Full autonomy within guardrails.
    AlwaysOn = 4,
}

impl AgentActivityLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Moderate => "moderate",
            Self::Active => "active",
            Self::AlwaysOn => "always_on",
        }
    }

    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" | "0" => Some(Self::Off),
            "minimal" | "1" => Some(Self::Minimal),
            "moderate" | "2" => Some(Self::Moderate),
            "active" | "3" => Some(Self::Active),
            "always_on" | "alwayson" | "4" => Some(Self::AlwaysOn),
            _ => None,
        }
    }

    /// Periodic sync interval in seconds for this level.
    /// Returns None for Off (manual-only).
    pub fn sync_interval_secs(self) -> Option<u64> {
        match self {
            Self::Off => None,
            Self::Minimal => Some(86_400), // 24h
            Self::Moderate => Some(3_600), // 1h
            Self::Active => Some(600),     // 10min
            Self::AlwaysOn => Some(60),    // 1min
        }
    }

    /// Whether heartbeat inference should run at this level.
    pub fn heartbeat_enabled(self) -> bool {
        matches!(self, Self::Moderate | Self::Active | Self::AlwaysOn)
    }

    /// Whether subconscious background reasoning should run.
    pub fn subconscious_enabled(self) -> bool {
        matches!(self, Self::Active | Self::AlwaysOn)
    }

    /// Per-background-cycle token budget. None = unlimited.
    pub fn token_budget_per_cycle(self) -> Option<u64> {
        match self {
            Self::Off => Some(0),
            Self::Minimal => Some(100_000),
            Self::Moderate => Some(500_000),
            Self::Active => Some(2_000_000),
            Self::AlwaysOn => None,
        }
    }

    /// Estimated monthly cost range (min, max) in USD for display.
    pub fn estimated_monthly_cost_range(self) -> (f64, f64) {
        match self {
            Self::Off => (0.0, 0.0),
            Self::Minimal => (0.10, 0.50),
            Self::Moderate => (1.0, 5.0),
            Self::Active => (5.0, 20.0),
            Self::AlwaysOn => (20.0, 100.0),
        }
    }
}

impl Default for AgentActivityLevel {
    fn default() -> Self {
        Self::Moderate
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_moderate() {
        assert_eq!(AgentActivityLevel::default(), AgentActivityLevel::Moderate);
    }

    #[test]
    fn from_str_round_trips() {
        for level in [
            AgentActivityLevel::Off,
            AgentActivityLevel::Minimal,
            AgentActivityLevel::Moderate,
            AgentActivityLevel::Active,
            AgentActivityLevel::AlwaysOn,
        ] {
            let parsed = AgentActivityLevel::from_str_opt(level.as_str()).unwrap();
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn serde_repr_round_trips() {
        for level in [
            AgentActivityLevel::Off,
            AgentActivityLevel::Minimal,
            AgentActivityLevel::Moderate,
            AgentActivityLevel::Active,
            AgentActivityLevel::AlwaysOn,
        ] {
            let json = serde_json::to_string(&level).unwrap();
            let parsed: AgentActivityLevel = serde_json::from_str(&json).unwrap();
            assert_eq!(parsed, level);
        }
    }

    #[test]
    fn sync_interval_none_for_off() {
        assert_eq!(AgentActivityLevel::Off.sync_interval_secs(), None);
        assert_eq!(
            AgentActivityLevel::Minimal.sync_interval_secs(),
            Some(86_400)
        );
    }

    #[test]
    fn heartbeat_disabled_for_low_levels() {
        assert!(!AgentActivityLevel::Off.heartbeat_enabled());
        assert!(!AgentActivityLevel::Minimal.heartbeat_enabled());
        assert!(AgentActivityLevel::Moderate.heartbeat_enabled());
    }
}
