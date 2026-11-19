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
    /// Wall-clock limit, in seconds, for one run of a `shell` cron job. The
    /// command is killed and the run fails as timed out when it passes.
    /// Defaults to 120; `0` is treated as 1.
    #[serde(default = "default_shell_timeout_secs")]
    pub shell_timeout_secs: u64,
}

/// Default for [`CronConfig::shell_timeout_secs`].
pub const DEFAULT_SHELL_TIMEOUT_SECS: u64 = 120;

fn default_shell_timeout_secs() -> u64 {
    DEFAULT_SHELL_TIMEOUT_SECS
}

impl CronConfig {
    /// The shell-job timeout to enforce: [`Self::shell_timeout_secs`], at
    /// least one second.
    pub fn shell_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.shell_timeout_secs.max(1))
    }
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
            shell_timeout_secs: default_shell_timeout_secs(),
        }
    }
}

#[cfg(test)]
#[path = "cron_tests.rs"]
mod tests;
