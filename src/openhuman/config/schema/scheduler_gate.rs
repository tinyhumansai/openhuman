//! Scheduler-gate configuration — controls when background AI work runs.
//!
//! Consumed by [`crate::openhuman::scheduler_gate`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerGateMode {
    /// Decide based on power + CPU + deployment-mode signals.
    Auto,
    /// Always run background AI flat-out (server / power-user setting).
    AlwaysOn,
    /// Never run background AI. User can still trigger work explicitly.
    Off,
}

impl SchedulerGateMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::AlwaysOn => "always_on",
            Self::Off => "off",
        }
    }
}

impl Default for SchedulerGateMode {
    fn default() -> Self {
        Self::Auto
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SchedulerGateConfig {
    /// Top-level mode — `auto` (default), `always_on`, or `off`.
    #[serde(default)]
    pub mode: SchedulerGateMode,

    /// Battery charge floor in `auto` mode, 0.0..=1.0. Below this and not on
    /// AC, the gate throttles. Default: 0.80.
    #[serde(default = "default_battery_floor")]
    pub battery_floor: f32,

    /// CPU busy threshold (recent global usage, 0..100). Above this, the gate
    /// throttles even when plugged in. Default: 70.0 (i.e. <30% headroom).
    #[serde(default = "default_cpu_busy_threshold")]
    pub cpu_busy_threshold_pct: f32,

    /// In `Throttled` mode, sleep this many ms before each LLM-bound job to
    /// serialise workers and let the host catch up. Default: 30_000 (30s).
    #[serde(default = "default_throttled_backoff_ms")]
    pub throttled_backoff_ms: u64,

    /// In `Paused` mode, re-check the policy every this many ms so workers
    /// resume promptly when the user toggles the gate back on. Default:
    /// 60_000 (60s).
    #[serde(default = "default_paused_poll_ms")]
    pub paused_poll_ms: u64,

    /// Hard CPU ceiling (recent global usage, 0..100). When the host CPU
    /// climbs above this in `auto` mode, the gate flips to
    /// `Paused { CpuPressure }` rather than just `Throttled` — every
    /// background LLM call is held until the host calms down. Distinct
    /// from `cpu_busy_threshold_pct`, which only triggers `Throttled`.
    /// Default: 95.0.
    #[serde(default = "default_cpu_severe_pct")]
    pub cpu_severe_pct: f32,

    /// When `true`, `auto` mode only runs background LLM work while the
    /// laptop is on AC power. On battery the gate flips to
    /// `Paused { OnBattery }` — no background inference at all,
    /// regardless of charge level.
    ///
    /// Default `false` to preserve the prior behavior (battery-floor
    /// based throttling). Power-conscious users who never want
    /// background inference on battery can flip this on.
    #[serde(default)]
    pub require_ac_power: bool,
}

fn default_battery_floor() -> f32 {
    0.80
}
fn default_cpu_busy_threshold() -> f32 {
    70.0
}
fn default_throttled_backoff_ms() -> u64 {
    30_000
}
fn default_paused_poll_ms() -> u64 {
    60_000
}
fn default_cpu_severe_pct() -> f32 {
    95.0
}

impl Default for SchedulerGateConfig {
    fn default() -> Self {
        Self {
            mode: SchedulerGateMode::default(),
            battery_floor: default_battery_floor(),
            cpu_busy_threshold_pct: default_cpu_busy_threshold(),
            throttled_backoff_ms: default_throttled_backoff_ms(),
            paused_poll_ms: default_paused_poll_ms(),
            cpu_severe_pct: default_cpu_severe_pct(),
            require_ac_power: false,
        }
    }
}
