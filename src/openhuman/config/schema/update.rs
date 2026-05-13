//! Auto-update configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// How `update.run` should complete after staging a new binary.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UpdateRestartStrategy {
    /// Request an in-process self-restart immediately after staging.
    SelfReplace,
    /// Stage the new binary and leave restart to an external supervisor.
    Supervisor,
}

impl Default for UpdateRestartStrategy {
    fn default() -> Self {
        Self::SelfReplace
    }
}

/// Configuration for periodic self-update checks against GitHub Releases.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct UpdateConfig {
    /// Enable periodic update checks. Defaults to `true`.
    #[serde(default = "default_update_enabled")]
    pub enabled: bool,

    /// Interval in minutes between update checks. Defaults to 60 (1 hour).
    /// Minimum enforced at runtime is 10 minutes.
    #[serde(default = "default_update_interval_minutes")]
    pub interval_minutes: u32,

    /// How `update.run` should handle restart after staging a new binary.
    #[serde(default)]
    pub restart_strategy: UpdateRestartStrategy,

    /// Whether bearer-authenticated RPC clients may invoke mutating update
    /// methods (`update.apply`, `update.run`).
    #[serde(default = "default_rpc_mutations_enabled")]
    pub rpc_mutations_enabled: bool,
}

fn default_update_enabled() -> bool {
    true
}

fn default_update_interval_minutes() -> u32 {
    60
}

fn default_rpc_mutations_enabled() -> bool {
    true
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            enabled: default_update_enabled(),
            interval_minutes: default_update_interval_minutes(),
            restart_strategy: UpdateRestartStrategy::default(),
            rpc_mutations_enabled: default_rpc_mutations_enabled(),
        }
    }
}
