//! Heartbeat and cron configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Heartbeat configuration — periodic background loop that evaluates
/// HEARTBEAT.md tasks against workspace state using local model inference.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HeartbeatConfig {
    /// Enable the heartbeat loop.
    pub enabled: bool,
    /// Tick interval in minutes (minimum 5).
    pub interval_minutes: u32,
    /// Enable subconscious inference (local model evaluation).
    /// When false, the heartbeat only counts tasks without reasoning.
    #[serde(default)]
    pub inference_enabled: bool,
    /// Maximum token budget for the situation report (default 40k).
    #[serde(default = "default_context_budget")]
    pub context_budget_tokens: u32,
}

fn default_context_budget() -> u32 {
    40_000
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_minutes: 5,
            inference_enabled: true,
            context_budget_tokens: default_context_budget(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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
