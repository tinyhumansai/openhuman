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

    /// Soft compaction trigger as a 0-100 percentage of the model's
    /// context window. When utilization crosses this, the pipeline runs
    /// microcompact and (if that doesn't free enough) summarization.
    /// Defaults to 90 to match the long-standing hardcoded threshold.
    #[serde(default = "default_compaction_trigger_pct")]
    pub compaction_trigger_pct: u8,

    /// Hard limit as a 0-100 percentage. Above this and with the
    /// compaction circuit breaker tripped, the guard returns
    /// `ContextExhausted` so the agent aborts the turn rather than
    /// sending an oversized request. Defaults to 95.
    #[serde(default = "default_hard_limit_pct")]
    pub hard_limit_pct: u8,

    /// Token budget reserved for the model's output. Subtracted from the
    /// available budget when deciding how aggressively to reduce the
    /// prompt. Defaults to 10_000 — large enough for a comfortable
    /// agentic response without eating too much of the window.
    #[serde(default = "default_reserve_output_tokens")]
    pub reserve_output_tokens: u64,

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

fn default_compaction_trigger_pct() -> u8 {
    90
}

fn default_hard_limit_pct() -> u8 {
    95
}

fn default_reserve_output_tokens() -> u64 {
    10_000
}

fn default_microcompact_keep_recent() -> usize {
    crate::openhuman::context::DEFAULT_KEEP_RECENT_TOOL_RESULTS
}

fn default_tool_result_budget_bytes() -> usize {
    crate::openhuman::context::DEFAULT_TOOL_RESULT_BUDGET_BYTES
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            microcompact_enabled: default_true(),
            autocompact_enabled: default_true(),
            compaction_trigger_pct: default_compaction_trigger_pct(),
            hard_limit_pct: default_hard_limit_pct(),
            reserve_output_tokens: default_reserve_output_tokens(),
            microcompact_keep_recent: default_microcompact_keep_recent(),
            tool_result_budget_bytes: default_tool_result_budget_bytes(),
            session_memory: SessionMemoryConfig::default(),
            summarizer_model: None,
        }
    }
}
