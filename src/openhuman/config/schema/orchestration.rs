//! Orchestration configuration.
//!
//! The reasoning/wake graph runs server-side now, so the device-side config is a
//! single opt-out: whether to ingest tiny.place harness session DMs and forward
//! them to the hosted orchestration brain.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct OrchestrationConfig {
    /// Ingest inbound tiny.place harness session DMs, forward them to the hosted
    /// brain (`POST /orchestration/v1/events`), run the device tail (effect
    /// executor, world-diff uploader, health probe), and render the hosted read
    /// surface. When `false` the device does none of this. Default: `true`.
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
