//! Orchestration configuration — controls the tiny.place harness session
//! ingest layer.
//!
//! Consumed by [`crate::openhuman::orchestration`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OrchestrationConfig {
    /// Ingest inbound tiny.place harness session DMs into the orchestration
    /// store. Default: `true`.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

impl Default for OrchestrationConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
        }
    }
}
