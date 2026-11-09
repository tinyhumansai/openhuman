//! Cron scheduler configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The `[cron]` config block.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct CronConfig {
    #[serde(default = "default_cron_enabled")]
    pub enabled: bool,
    #[serde(default = "default_cron_max_run_history")]
    pub max_run_history: usize,
    /// Operator-owned mode for hosted deployments: agent jobs bound to the
    /// conversation that created them, plus the memory domain's two
    /// code-owned maintenance jobs. Shell and user-owned flow jobs are refused
    /// at add, update, run and dispatch; memory jobs are seeded internally and
    /// count toward [`Self::max_active_jobs`]. Delivery is limited to `origin`
    /// and `none`; recurring jobs must keep to [`Self::min_interval_secs`].
    #[serde(default)]
    pub agent_jobs_only: bool,
    /// With `agent_jobs_only`: the shortest gap allowed between runs of a
    /// recurring job.
    #[serde(default = "default_min_interval_secs")]
    pub min_interval_secs: u64,
    /// With `agent_jobs_only`: the most enabled jobs the store may hold for
    /// one profile, counted in the store.
    #[serde(default = "default_max_active_jobs")]
    pub max_active_jobs: u32,
}

fn default_min_interval_secs() -> u64 {
    300
}

fn default_max_active_jobs() -> u32 {
    20
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
            agent_jobs_only: false,
            min_interval_secs: default_min_interval_secs(),
            max_active_jobs: default_max_active_jobs(),
        }
    }
}
