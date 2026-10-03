//! Context management configuration.
//!
//! Knobs for the global `crates/openhuman-core/src/agent/context/` module — budget
//! thresholds, summarization trigger percentages and microcompact behavior. Wired into the root
//! [`super::Config`] as the `context` section; env overrides live in
//! [`super::load`].

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Top-level context-management config. All fields are optional in
/// `config.toml` and fall back to the defaults shipped in
/// [`ContextConfig::default`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ContextConfig {
    /// Master switch. When `false`, [`crate::agent::context::ContextManager`]
    /// skips every reduction stage and the summarizer is never invoked.
    /// Useful for tests and diagnostics; not recommended for production.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Enable stage 3 (microcompact) — clearing older `ToolResults`
    /// payloads to free tokens before falling back to summarization.
    #[serde(default = "default_true")]
    pub microcompact_enabled: bool,

    /// Enable stage 4 (autocompact) — install the TinyAgents summarization
    /// middleware when the transcript approaches the model context window.
    #[serde(default = "default_true")]
    pub autocompact_enabled: bool,

    /// How many of the most-recent `ToolResults` envelopes microcompact
    /// leaves untouched when it runs. Older envelopes are cleared first.
    #[serde(default = "default_microcompact_keep_recent")]
    pub microcompact_keep_recent: usize,

    /// Maximum byte length of a single tool-result body before the
    /// TinyAgents tool-output middleware budget stage truncates it.
    /// `0` disables the cap. Applied inline at tool-execution time
    /// before the result enters history, so it is cache-safe.
    ///
    /// **Migration note:** this field used to live on
    /// [`super::AgentConfig::tool_result_budget_bytes`]. It has moved
    /// here because it is logically a context-reduction knob. A
    /// compatibility `#[serde(alias)]` on `AgentConfig` keeps existing
    /// `config.toml` files parsing cleanly during the transition.
    #[serde(default = "default_tool_result_budget_bytes")]
    pub tool_result_budget_bytes: usize,

    /// Tool results larger than this **token** count trigger the
    /// `summarizer` sub-agent (orchestrator session only). The summarizer
    /// compresses the payload into a dense note that preserves
    /// identifiers and key facts, and the compressed summary replaces
    /// the raw payload before it enters agent history. Default: 4000 tokens.
    /// Set to 0 to disable.
    ///
    /// Token count is estimated as `chars / 4` (a rough heuristic). Pairs with
    /// [`Self::summarizer_max_payload_tokens`] which caps the upper end
    /// (paying for an LLM call on a multi-million-token blob makes no
    /// economic sense, so above the cap the existing
    /// [`Self::tool_result_budget_bytes`] truncation handles it instead).
    #[serde(
        default = "default_summarizer_payload_threshold_tokens",
        alias = "summarizer_payload_threshold_bytes"
    )]
    pub summarizer_payload_threshold_tokens: usize,

    /// Hard cap on payload size (in **tokens**) above which summarization
    /// is skipped entirely and the existing
    /// [`Self::tool_result_budget_bytes`] truncation path takes over.
    /// Default: `2_000_000` tokens (above the context window of every
    /// model we ship against — a payload this big can't be summarized
    /// cost-effectively).
    #[serde(
        default = "default_summarizer_max_payload_tokens",
        alias = "summarizer_max_payload_bytes"
    )]
    pub summarizer_max_payload_tokens: usize,

    /// Override for the model used by the summarizer when autocompaction
    /// fires. `None` (the default) means "use the caller's current
    /// model"; set this to a cheaper/faster model to reduce the cost of
    /// summarization on long sessions.
    #[serde(default)]
    pub summarizer_model: Option<String>,

    /// When `true`, the agent loop asks tools to render their results as
    /// markdown instead of JSON before they enter LLM context. Tools that
    /// support it populate `ToolResult::markdown_formatted`; the harness
    /// prefers that field over the JSON fallback. Markdown is materially
    /// cheaper than JSON in tokens, especially on tool-heavy loops.
    /// Default: `true` — opt out per-deployment via config or env if a
    /// downstream consumer expects strict JSON tool output.
    #[serde(default = "default_true")]
    pub prefer_markdown_tool_output: bool,

    /// Switch for tokenjuice tool-output compaction (Stage 1a). When `true`,
    /// large tool outputs are handled by the TinyJuice module *before* the
    /// [`Self::tool_result_budget_bytes`] byte cap and before they enter history.
    ///
    /// With `[tokenjuice] repl_handle_enabled` (the default) a large result is
    /// stored in the CCR cache and replaced by a stats line, a short head and a
    /// handle. The model reads that, and queries the rest with `juice_find`,
    /// `juice_extract` and `juice_summarize` (registered only while this switch
    /// is on) or takes the whole original with `juice_retrieve`. No model
    /// call is involved, so it cannot stall on a slow summarizer. With
    /// `repl_handle_enabled = false` the result is instead compressed to one
    /// blob with a `⟦tj:<hash>⟧` marker for `juice_retrieve`.
    ///
    /// **On by default.** It was off from the 2026-09 latency work, when a
    /// compacted view cost a retrieval round trip more often than it saved
    /// context, every curated belt paid for the retrieve schema, and the LLM
    /// summary could stall a turn. The handle path avoids the stall and gives
    /// the model queries instead of a blind retrieve. The per-tool char cap and
    /// the shared byte backstop apply either way. Opt out with
    /// `compaction_enabled = false` or `OPENHUMAN_COMPACTION=0`.
    #[serde(default = "default_true")]
    pub compaction_enabled: bool,

    /// Absolute token count at which context compaction (the summarization
    /// step) fires, overriding the default of min(80% of the model's context
    /// window, 350k tokens). Also enables compaction for a model whose window
    /// is unknown.
    ///
    /// For benchmarks and debugging that need compaction to happen early;
    /// leave unset in normal use. `None` or `0` means no override. Env:
    /// `OPENHUMAN_COMPACTION_TRIGGER_TOKENS`.
    #[serde(default)]
    pub compaction_trigger_tokens: Option<u64>,

    /// How a compaction writes its checkpoint. `task_state` (the default) is a
    /// typed task state: facts copied from tool calls (original task, files
    /// modified and read, recent commands and errors) plus one structured
    /// model call, over a 20k-token verbatim tail. `summary` is the earlier
    /// free-form LLM summary over the last 8 messages. Env:
    /// `OPENHUMAN_COMPACTION_STRATEGY`.
    #[serde(default)]
    pub compaction_strategy: CompactionStrategy,
}

/// The compaction knobs a turn carries: the trigger override and the
/// checkpoint strategy. See [`ContextConfig::compaction_settings`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionSettings {
    /// Absolute trigger override (`None`: the window-relative default).
    pub trigger_tokens: Option<u64>,
    /// How a compaction writes its checkpoint.
    pub strategy: CompactionStrategy,
}

impl ContextConfig {
    /// The compaction knobs for a turn; a `0` trigger reads as no override.
    #[must_use]
    pub fn compaction_settings(&self) -> CompactionSettings {
        CompactionSettings {
            trigger_tokens: self.compaction_trigger_tokens.filter(|t| *t > 0),
            strategy: self.compaction_strategy,
        }
    }
}

/// How a context compaction writes its checkpoint. See
/// [`ContextConfig::compaction_strategy`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompactionStrategy {
    /// Typed task state (TinyAgents `TaskStateSummarizer`) over a
    /// token-budgeted tail. Chosen by the openhuman-benchmarks compaction eval.
    #[default]
    TaskState,
    /// Free-form LLM summary (TinyAgents `ModelSummarizer`) over the last
    /// `keep_last` messages.
    Summary,
}

impl CompactionStrategy {
    /// Parses a config or env value (`task_state`/`typed`, `summary`).
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('-', "_").as_str() {
            "task_state" | "typed" | "s4" => Some(Self::TaskState),
            "summary" | "free_form" | "s0" => Some(Self::Summary),
            _ => None,
        }
    }
}

fn default_enabled() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_microcompact_keep_recent() -> usize {
    crate::agent::context::DEFAULT_KEEP_RECENT_TOOL_RESULTS
}

fn default_tool_result_budget_bytes() -> usize {
    crate::agent::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES
}

fn default_summarizer_payload_threshold_tokens() -> usize {
    // Re-enabled at 4000 tokens after the recursive-dispatch root cause
    // was fixed by the narrow sub-agent prompt built for the
    // summarizer archetype (which prevents it from seeing `spawn_subagent`
    // and thus cannot recurse). 0 would leave this entirely disabled.
    4000
}

fn default_summarizer_max_payload_tokens() -> usize {
    2_000_000
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            microcompact_enabled: default_true(),
            autocompact_enabled: default_true(),
            microcompact_keep_recent: default_microcompact_keep_recent(),
            tool_result_budget_bytes: default_tool_result_budget_bytes(),
            summarizer_payload_threshold_tokens: default_summarizer_payload_threshold_tokens(),
            summarizer_max_payload_tokens: default_summarizer_max_payload_tokens(),
            summarizer_model: None,
            prefer_markdown_tool_output: default_true(),
            compaction_enabled: default_true(),
            compaction_trigger_tokens: None,
            compaction_strategy: CompactionStrategy::default(),
        }
    }
}
