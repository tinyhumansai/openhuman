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

/// Which turns' reasoning a request replays (`[context] replay_reasoning`).
///
/// The config's own copy of the runtime policy, so the schema does not
/// re-export a TinyAgents type. It serializes with the same names
/// (`all` | `current_turn`), and the runtime reads it through
/// `From<ReasoningReplay>` where a run is configured.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReasoningReplay {
    /// Every assistant message keeps its reasoning blocks (today's behaviour).
    #[default]
    All,
    /// Only the current turn's reasoning is sent; reasoning from completed
    /// user turns is omitted from the request.
    CurrentTurn,
}

impl From<ReasoningReplay> for tinyagents_harness::reasoning_replay::ReasoningReplay {
    fn from(replay: ReasoningReplay) -> Self {
        match replay {
            ReasoningReplay::All => Self::All,
            ReasoningReplay::CurrentTurn => Self::CurrentTurn,
        }
    }
}

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
    /// Verbatim tail, in tokens, an in-turn compaction keeps after its
    /// summary. Unset keeps 20,000 (capped at a fifth of the context window).
    /// An embedder sets it per session (`AgentSpec::keep_recent_tokens`). Env:
    /// `OPENHUMAN_CONTEXT_KEEP_RECENT_TOKENS` (`0` clears it).
    #[serde(default)]
    pub keep_recent_tokens: Option<u32>,
    /// Replace a tool result older than this many user turns with a one-line
    /// note, at the start of each run. Unset (or `0`) keeps every result, as
    /// before. Pinned on each thread when it is created: a running thread keeps
    /// the value it started with. Env: `OPENHUMAN_CONTEXT_TOOL_RESULT_MAX_AGE_TURNS`.
    #[serde(default)]
    pub tool_result_max_age_turns: Option<u32>,
    /// Tools whose results never expire, by exact name. Only read while
    /// `tool_result_max_age_turns` is set. Env:
    /// `OPENHUMAN_CONTEXT_TOOL_RESULT_EXEMPT_TOOLS` (comma separated).
    #[serde(default)]
    pub tool_result_exempt_tools: Vec<String>,
    /// Which turns' reasoning a request replays: `all` (today's behaviour) or
    /// `current_turn`. The stored transcript always keeps every block. Pinned
    /// on each thread when it is created. Env: `OPENHUMAN_CONTEXT_REPLAY_REASONING`.
    #[serde(default)]
    pub replay_reasoning: ReasoningReplay,
    /// Drop a recorded tool declaration the host no longer offers instead of
    /// carrying it forward for the life of the thread. Env:
    /// `OPENHUMAN_CONTEXT_PRUNE_RETIRED_TOOLS`.
    #[serde(default)]
    pub prune_retired_tools: bool,
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

/// Per-session overrides of the `[context]` keys an embedder may set for one
/// agent or one thread. A field left `None` keeps the config's value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextOverrides {
    /// Replaces [`ContextSettings::keep_recent_tokens`].
    pub keep_recent_tokens: Option<u32>,
    /// Replaces [`ContextSettings::tool_result_max_age_turns`].
    pub tool_result_max_age_turns: Option<u32>,
    /// Replaces [`ContextSettings::tool_result_exempt_tools`].
    pub tool_result_exempt_tools: Option<Vec<String>>,
    /// Replaces [`ContextSettings::replay_reasoning`].
    pub replay_reasoning: Option<ReasoningReplay>,
    /// Replaces [`ContextSettings::prune_retired_tools`].
    pub prune_retired_tools: Option<bool>,
}

impl ContextOverrides {
    /// Writes every override that is set into `settings`.
    pub fn apply_to(&self, settings: &mut ContextSettings) {
        if let Some(tokens) = self.keep_recent_tokens {
            settings.keep_recent_tokens = Some(tokens);
        }
        if let Some(age) = self.tool_result_max_age_turns {
            settings.tool_result_max_age_turns = Some(age);
        }
        if let Some(tools) = &self.tool_result_exempt_tools {
            settings.tool_result_exempt_tools = tools.clone();
        }
        if let Some(replay) = self.replay_reasoning {
            settings.replay_reasoning = replay;
        }
        if let Some(prune) = self.prune_retired_tools {
            settings.prune_retired_tools = prune;
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
    /// Verbatim tail an in-turn compaction keeps, in tokens; unset keeps
    /// 20,000. Env: `OPENHUMAN_CONTEXT_KEEP_RECENT_TOKENS`.
    keep_recent_tokens: Option<u32>,
    /// Replace a tool result older than this many user turns with a one-line
    /// note; unset or `0` keeps every result. Env:
    /// `OPENHUMAN_CONTEXT_TOOL_RESULT_MAX_AGE_TURNS`.
    tool_result_max_age_turns: Option<u32>,
    /// Tools whose results never expire. Env:
    /// `OPENHUMAN_CONTEXT_TOOL_RESULT_EXEMPT_TOOLS`.
    tool_result_exempt_tools: Vec<String>,
    /// `all` or `current_turn`. Env: `OPENHUMAN_CONTEXT_REPLAY_REASONING`.
    replay_reasoning: ReasoningReplaySchema,
    /// Drop recorded tool declarations the host no longer offers. Env:
    /// `OPENHUMAN_CONTEXT_PRUNE_RETIRED_TOOLS`.
    prune_retired_tools: bool,
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
enum ReasoningReplaySchema {
    All,
    CurrentTurn,
}

#[derive(schemars::JsonSchema)]
#[schemars(rename_all = "snake_case")]
enum ToolRenderingSchema {
    Full,
    Markers,
}
