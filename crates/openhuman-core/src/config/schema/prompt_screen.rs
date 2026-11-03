//! Prompt screen configuration: whether untrusted input is checked for prompt
//! injection by TinySecurity before a turn, a tool result or an inbound
//! message is processed.
//!
//! The screen fails closed. With the default [`PromptScreenMode::Enforce`],
//! input is held for review whenever TinySecurity cannot give a verdict: the
//! module is not compiled in (no `security-module` feature), it is not loaded,
//! it errors, or it times out. A host that screens input somewhere else, or that
//! ships without the TinySecurity module on purpose, opts out with
//! [`PromptScreenMode::Off`]; nothing turns the screen off implicitly.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Whether the prompt screen runs.
///
/// Serializes as snake_case (`enforce` / `off`). A config missing the
/// `[prompt_screen]` block, or its `mode` key, deserializes to
/// [`PromptScreenMode::Enforce`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PromptScreenMode {
    /// Screen every input through TinySecurity, holding it for review when no
    /// verdict can be had.
    #[default]
    Enforce,
    /// Do not screen: every input is allowed without consulting TinySecurity.
    /// For a host that screens input itself or runs without the module.
    Off,
}

/// The `[prompt_screen]` config block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct PromptScreenConfig {
    /// Whether the screen runs. Missing key → [`PromptScreenMode::Enforce`].
    pub mode: PromptScreenMode,
}

impl PromptScreenConfig {
    /// The screen turned off: [`PromptScreenMode::Off`].
    #[must_use]
    pub fn off() -> Self {
        Self {
            mode: PromptScreenMode::Off,
        }
    }

    /// Whether input is screened. False only when the config says
    /// [`PromptScreenMode::Off`].
    #[must_use]
    pub fn enforces(&self) -> bool {
        self.mode == PromptScreenMode::Enforce
    }
}

#[cfg(test)]
#[path = "prompt_screen_tests.rs"]
mod tests;
