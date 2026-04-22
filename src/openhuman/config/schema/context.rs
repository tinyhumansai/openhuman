//! Context management configuration.
//!
//! Knobs for the global `src/openhuman/context/` module — budget
//! thresholds, summarization trigger percentages, microcompact behavior,
//! and the session-memory extraction cadence. Wired into the root
//! [`super::Config`] as the `context` section; env overrides live in
//! [`super::load`].

use crate::openhuman::context::session_memory::SessionMemoryConfig;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Top-level context-management config. All fields are optional in
/// `config.toml` and fall back to the defaults shipped in
/// [`ContextConfig::default`].
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ContextConfig {
    /// Master switch. When `false`, [`crate::openhuman::context::ContextManager`]
    /// skips every reduction stage and the summarizer is never invoked.
    /// Useful for tests and diagnostics; not recommended for production.
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    /// Enable stage 3 (microcompact) — clearing older `ToolResults`
    /// payloads to free tokens before falling back to summarization.
    #[serde(default = "default_true")]
    pub microcompact_enabled: bool,

    /// Enable stage 4 (autocompact) — dispatch the summarizer when
    /// microcompact cannot free enough tokens. Disabling this makes the
    /// pipeline return `PipelineOutcome::NoOp` at the soft threshold and
    /// trust the caller to surface the situation via the guard.
    #[serde(default = "default_true")]
    pub autocompact_enabled: bool,

    /// How many of the most-recent `ToolResults` envelopes microcompact
    /// leaves untouched when it runs. Older envelopes are cleared first.
    #[serde(default = "default_microcompact_keep_recent")]
    pub microcompact_keep_recent: usize,

    /// Maximum byte length of a single tool-result body before the
    /// context pipeline's tool-result budget stage truncates it.
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
    /// the raw payload before it enters agent history. Set to `0` to
    /// disable summarization entirely (the default). Set to any value
    /// `> 0` to enable summarization once a payload crosses that token
    /// threshold.
    ///
    /// Token count is estimated as `chars / 4` (the same heuristic used
    /// by `tree_summarizer::estimate_tokens`). Pairs with
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

    /// Session-memory extraction thresholds (stage 5 of the pipeline).
    #[serde(default)]
    pub session_memory: SessionMemoryConfig,

    /// Override for the model used by the summarizer when autocompaction
    /// fires. `None` (the default) means "use the caller's current
    /// model"; set this to a cheaper/faster model to reduce the cost of
    /// summarization on long sessions.
    #[serde(default)]
    pub summarizer_model: Option<String>,
}

fn default_enabled() -> bool {
    true
}

fn default_true() -> bool {
    true
}

fn default_microcompact_keep_recent() -> usize {
    crate::openhuman::context::DEFAULT_KEEP_RECENT_TOOL_RESULTS
}

fn default_tool_result_budget_bytes() -> usize {
    crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES
}

fn default_summarizer_payload_threshold_tokens() -> usize {
    // Disabled: 0 short-circuits the payload_summarizer wiring in the
    // agent builder (see session/builder.rs `> 0` guard). The summarizer
    // sub-agent was being invoked recursively in some flows; keep off
    // until that's root-caused.
    0
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
            session_memory: SessionMemoryConfig::default(),
            summarizer_model: None,
        }
    }
}
