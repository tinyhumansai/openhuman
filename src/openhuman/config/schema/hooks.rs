//! `[hooks]` — host-level switches for the configurable hook system.
//!
//! The hooks themselves are **not** configured here. They live in `hooks.json`
//! files discovered across four layers
//! ([`crate::openhuman::hooks::config`]), because a hook set belongs with the
//! repository it guards and has to be readable by a human who has never seen
//! this config file. This block only carries the decisions that belong to the
//! host: whether the system runs at all, and how long a hook that names no
//! timeout of its own may take.

use serde::{Deserialize, Serialize};

/// Host-level hook settings.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct HooksConfig {
    /// Master switch. Off means no `hooks.json` is read and the harness bridge
    /// is not installed, so an unconfigured host pays nothing per tool call.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Seconds a hook may run when its definition names no `timeout`.
    #[serde(default = "default_timeout_secs")]
    pub default_timeout_secs: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_timeout_secs() -> u64 {
    30
}

impl Default for HooksConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            default_timeout_secs: default_timeout_secs(),
        }
    }
}
