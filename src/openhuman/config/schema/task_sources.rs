//! Task-sources configuration — app-level defaults for the
//! [`crate::openhuman::integrations::task_sources`] domain.
//!
//! Per-source records (provider + filter + schedule) live in the
//! domain's SQLite store, not here. This block only carries the master
//! switch and the defaults applied when a new source is created without
//! explicit values.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct TaskSourcesConfig {
    /// Master switch. When `false`, the periodic poll skips every source
    /// (manual `task_sources_fetch` RPCs still work).
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Default poll interval (seconds) for a new source that doesn't
    /// specify one. Default: 1800 (30 minutes).
    #[serde(default = "default_interval_secs")]
    pub default_interval_secs: u64,

    /// Default per-fetch task cap for a new source. Default: 25.
    #[serde(default = "default_max_tasks")]
    pub max_tasks_per_fetch: u32,

    /// When `true` (default), a new source defaults to the proactive
    /// target (todo card + triage turn); when `false`, todo-only.
    #[serde(default = "default_auto_proactive")]
    pub auto_proactive: bool,
}

fn default_enabled() -> bool {
    true
}
fn default_interval_secs() -> u64 {
    1800
}
fn default_max_tasks() -> u32 {
    25
}
fn default_auto_proactive() -> bool {
    true
}

impl Default for TaskSourcesConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            default_interval_secs: default_interval_secs(),
            max_tasks_per_fetch: default_max_tasks(),
            auto_proactive: default_auto_proactive(),
        }
    }
}

#[cfg(test)]
#[path = "task_sources_tests.rs"]
mod tests;
