//! TinyComputer model selection: the decision model that picks each step,
//! and the reasoning models that plan a task and rescue a failed step.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The model family that decides each desktop or browser step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DecisionModel {
    /// Jev, through TinyHumans (signed in) or the user's OpenRouter key.
    #[default]
    Jev,
    /// OpenJev, the open Jev endpoint; needs an `openjev` credential.
    OpenJev,
    /// Levanto Sage; needs a `sage` credential.
    Sage,
}

impl DecisionModel {
    /// The stable wire spelling, as the module config and the UI use it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Jev => "jev",
            Self::OpenJev => "open_jev",
            Self::Sage => "sage",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ComputerConfig {
    /// Which decision model TinyComputer asks for each step.
    pub decision_model: DecisionModel,
    /// Sage's fast mode: quicker, less deliberate decisions.
    pub sage_fast: bool,
    /// Model that turns a plain-language task into a flow. `None` keeps the
    /// module's default.
    pub planner_model: Option<String>,
    /// Model a failed step is handed to for guidance before the task fails.
    /// `None` keeps the module's default.
    pub rescue_model: Option<String>,
    /// How many times one task may be rescued; `0` turns rescue off and
    /// `None` keeps the module's default (5, also its maximum).
    pub max_rescues: Option<u32>,
    /// Let a browser task try a captcha or "verify you are human" check with
    /// Jev's image questions before it asks a person. Needs a TypeSafe key
    /// (the `typesafe` credential or `TYPESAFE_API_KEY`). Off by default.
    pub captcha_solver: bool,
    /// The image-reading Jev model the captcha solver asks. `None` keeps the
    /// module's default (`jev-preview`).
    pub captcha_model: Option<String>,
    /// Image challenges one try may submit before the task asks a person;
    /// `None` keeps the module's default (3). At most 10.
    pub max_captcha_attempts: Option<u32>,
    /// Record every decision of a browser task and keep each task's full
    /// report, and trace desktop commands, under `<workspace>/state/computer/`.
    /// Off by default: reports hold page text. `OPENHUMAN_COMPUTER_TRACE=1`
    /// turns it on for one run without editing the config.
    pub trace: bool,
}

#[cfg(test)]
#[path = "computer_tests.rs"]
mod tests;
