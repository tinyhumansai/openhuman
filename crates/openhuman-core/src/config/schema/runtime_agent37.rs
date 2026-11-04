//! `[runtime.agent37]` — how the agent37 sandbox backend creates its boxes.
//!
//! With `[security.sandbox] backend = "agent37"`, every sandboxed shell command
//! runs in a box the TinyBox module creates on the hosted agent37 service. The
//! service starts a box from a registered *template name*, not an OCI image
//! reference, so this is a separate setting from `[runtime.docker] image`:
//! agent37 rejects an inline image reference (for example the `alpine:3.20`
//! Docker default) with an HTTP 400, so forwarding the Docker image could never
//! create a box.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The agent37 template a box starts from when none is configured. It is a
/// template name the service already knows, not an OCI image reference.
pub const DEFAULT_AGENT37_TEMPLATE: &str = "agent37-base";

/// `[runtime.agent37]`: what each agent37 box is created from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct Agent37RuntimeConfig {
    /// The registered agent37 template name every box starts from. This names a
    /// template the service hosts; it is not an OCI image reference, and CPU and
    /// memory still come from `[runtime.docker]` (best effort — the service snaps
    /// them to fixed instance shapes).
    pub template: String,
}

impl Default for Agent37RuntimeConfig {
    fn default() -> Self {
        Self {
            template: DEFAULT_AGENT37_TEMPLATE.to_owned(),
        }
    }
}

#[cfg(test)]
#[path = "runtime_agent37_tests.rs"]
mod tests;
