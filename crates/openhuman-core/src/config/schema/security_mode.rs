//! Security mode configuration: whether the checks OpenHuman delegates to
//! TinySecurity run at all (`[security]`).
//!
//! With the default [`SecurityMode::Enforce`] every delegated check (the
//! prompt screen, action classification and approvals, path checks, the action
//! budget, egress privacy checks, redaction and the audit trail) asks the
//! module and fails closed whenever it cannot answer: not loaded, erroring or
//! slow. A host that runs its own controls, or that ships without the module,
//! turns them off with [`SecurityMode::Off`]; the checks then allow and
//! redaction passes content through unchanged. A build without the
//! `security-module` feature behaves as `off` whatever this says, because there
//! is no module to ask (`crate::security::switch`).
//!
//! This is not the daemon's [`SecurityConfig`](super::SecurityConfig), which
//! configures sandboxing, resource limits and the audit sink.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether the TinySecurity-delegated checks run.
///
/// Serializes as snake_case (`enforce` / `off`). A config missing the
/// `[security]` block, or its `mode` key, deserializes to
/// [`SecurityMode::Enforce`]. Any other value is a parse error.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SecurityMode {
    /// Ask TinySecurity, refusing whatever it cannot confirm.
    #[default]
    Enforce,
    /// Do not ask: every delegated check allows, and redaction passes content
    /// through. Implies `[prompt_screen] mode = "off"`.
    Off,
}

/// The `[security]` config block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct SecurityModeConfig {
    /// Whether the checks run. Missing key → [`SecurityMode::Enforce`].
    pub mode: SecurityMode,
}

impl SecurityModeConfig {
    /// The checks turned off: [`SecurityMode::Off`].
    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: SecurityMode::Off,
        }
    }

    /// Whether the checks run. False only when the config says
    /// [`SecurityMode::Off`].
    #[must_use]
    pub fn enforces(&self) -> bool {
        self.mode == SecurityMode::Enforce
    }
}

#[cfg(test)]
#[path = "security_mode_tests.rs"]
mod tests;
