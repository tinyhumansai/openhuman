//! The `[context]` table: TinyAgents-owned context settings, re-exported
//! through OpenHuman's config API, plus the summarizer settings OpenHuman adds
//! beside them.
//!
//! OpenHuman keeps its product-specific environment overlays in
//! [`super::load`], while the shared settings, defaults, parsing, and
//! serialization contract live with TinyAgents. [`ContextSettings`] flattens
//! TinyAgents' [`ContextConfig`] and adds OpenHuman's own keys to the same
//! table, so `config.context.<harness field>` keeps reading through `Deref`.

use serde::{Deserialize, Serialize};

pub use tinyagents_harness::config::{CompactionSettings, CompactionStrategy, ContextConfig};
use tinyagents_harness::summarization::ToolRendering;

/// The whole `[context]` table: TinyAgents' [`ContextConfig`] and the
/// compaction summarizer's OpenHuman-side settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextSettings {
    /// TinyAgents' context settings, flattened into the same table.
    #[serde(flatten)]
    pub harness: ContextConfig,
    /// How the compaction summarizer sees tool activity: `full` (every call
    /// and result) or `markers` (one `[used <tool>: …]` line per call, no
    /// results), for small summarization models. Env:
    /// `OPENHUMAN_CONTEXT_SUMMARIZER_TOOL_RENDERING`.
    #[serde(default)]
    pub summarizer_tool_rendering: ToolRendering,
    /// Output cap for each summary, sent as the request's `max_tokens`. Unset
    /// leaves the provider default. Env:
    /// `OPENHUMAN_CONTEXT_SUMMARIZER_MAX_OUTPUT_TOKENS` (`0` clears it).
    #[serde(default)]
    pub summarizer_max_output_tokens: Option<u32>,
}

impl ContextSettings {
    /// The summarizer settings a compaction applies.
    pub fn summarizer_settings(&self) -> SummarizerSettings {
        SummarizerSettings {
            tool_rendering: self.summarizer_tool_rendering,
            max_output_tokens: self.summarizer_max_output_tokens,
        }
    }
}

impl std::ops::Deref for ContextSettings {
    type Target = ContextConfig;

    fn deref(&self) -> &ContextConfig {
        &self.harness
    }
}

impl std::ops::DerefMut for ContextSettings {
    fn deref_mut(&mut self) -> &mut ContextConfig {
        &mut self.harness
    }
}

/// How the free-form compaction summarizer renders its input and caps its
/// output (`[context] summarizer_tool_rendering` /
/// `summarizer_max_output_tokens`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SummarizerSettings {
    /// How tool calls and results reach the summarizer.
    pub tool_rendering: ToolRendering,
    /// The summary's `max_tokens`; `None` leaves the provider default.
    pub max_output_tokens: Option<u32>,
}

/// JSON-schema view of TinyAgents' serialized context settings.
///
/// The harness type deliberately stays independent from `schemars`; this
/// mirror keeps OpenHuman's generated config schema complete without adding a
/// documentation dependency to the owning crate.
#[derive(schemars::JsonSchema)]
pub(crate) struct ContextConfigSchema {
    enabled: bool,
    microcompact_enabled: bool,
    autocompact_enabled: bool,
    microcompact_keep_recent: usize,
    tool_result_budget_bytes: usize,
    summarizer_payload_threshold_tokens: usize,
    summarizer_max_payload_tokens: usize,
    /// The model the context-compaction summarizer calls, on the turn's own
    /// provider route (same credentials and endpoint): on a BYOK OpenRouter
    /// route, `"scaledown/summarize"` calls that model on OpenRouter. Unset
    /// or blank, compaction uses the turn's model. Env:
    /// `OPENHUMAN_CONTEXT_SUMMARIZER_MODEL`.
    summarizer_model: Option<String>,
    /// How the summarizer sees tool activity: `full` or `markers`. Env:
    /// `OPENHUMAN_CONTEXT_SUMMARIZER_TOOL_RENDERING`.
    summarizer_tool_rendering: ToolRenderingSchema,
    /// The summary's `max_tokens`; unset leaves the provider default. Env:
    /// `OPENHUMAN_CONTEXT_SUMMARIZER_MAX_OUTPUT_TOKENS`.
    summarizer_max_output_tokens: Option<u32>,
    prefer_markdown_tool_output: bool,
    compaction_enabled: bool,
    compaction_trigger_tokens: Option<u64>,
    compaction_strategy: CompactionStrategySchema,
}

#[derive(schemars::JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum CompactionStrategySchema {
    TaskState,
    Summary,
}

#[derive(schemars::JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum ToolRenderingSchema {
    Full,
    Markers,
}
