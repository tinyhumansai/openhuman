//! [`ContextManager`] — the single per-session handle agents use to
//! manage their prompt and their in-flight conversation context.
//!
//! # What this owns
//!
//! 1. **System prompt assembly** — a default [`SystemPromptBuilder`]
//!    configured once at session start (usually
//!    `SystemPromptBuilder::with_defaults()`). Callers that need a
//!    different builder shape — sub-agent archetype sections, channel
//!    capabilities sections — pass their own via
//!    [`ContextManager::build_system_prompt_with`].
//!
//! 2. **Context bookkeeping** — a [`ContextStatsState`] with utilisation
//!    stats and tool-result budget config.
//!    Live history reduction/summarization moved to the
//!    tinyagents graph (`ContextCompressionMiddleware` +
//!    `MessageTrimMiddleware`, issue #4249); this manager no longer runs
//!    an in-turn summarizer.
//!

use super::stats::ContextStatsState;
use crate::agent::prompts::{PromptContext, SystemPromptBuilder};
use crate::config::ContextConfig;
use crate::inference::provider::BilledUsage;
use anyhow::Result;

/// Read-only snapshot of per-session context state. Returned by
/// [`ContextManager::stats`] for observability and the optional
/// `context.get_stats` RPC.
#[derive(Debug, Clone, Default)]
pub struct ContextStats {
    pub utilisation_pct: Option<u8>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_window: u64,
}

/// Per-session context manager. Constructed once by the agent harness
/// at session start; lives for the whole lifetime of the `Agent`.
pub struct ContextManager {
    stats_state: ContextStatsState,
    /// The default system-prompt builder used by
    /// [`ContextManager::build_system_prompt`]. Held by value so the
    /// agent's construction-time builder configuration survives the
    /// move into the manager.
    default_prompt_builder: SystemPromptBuilder,
    /// Whether the entire module is enabled. Useful for tests and
    /// debugging; see [`ContextConfig::enabled`]. Live history reduction
    /// now runs in the tinyagents graph (`ContextCompressionMiddleware` +
    /// `MessageTrimMiddleware`, issue #4249); this flag only gates the
    /// manager's own bookkeeping surfaces.
    enabled: bool,
    /// Per-tool-result byte cap applied inline at tool-execution time.
    /// Stored on the manager (rather than on the agent directly) so
    /// every caller that touches "what's in the model's context window"
    /// reads the same source of truth.
    tool_result_budget_bytes: usize,
    /// When `true`, the agent loop asks tools to populate
    /// `ToolResult::markdown_formatted` so the harness can hand the LLM
    /// markdown instead of JSON — significantly cheaper in the model
    /// context window. See [`ContextConfig::prefer_markdown_tool_output`].
    prefer_markdown_tool_output: bool,
    /// When `true`, native tool-output compaction (Stage 1a) runs in
    /// `OpenHumanSessionHost::execute_tool_call` before the byte cap. On by default; the
    /// kill-switch lives here so every caller reads one source of truth.
    /// See [`ContextConfig::compaction_enabled`].
    compaction_enabled: bool,
    /// Number of most-recent tool results kept verbatim by the microcompact
    /// middleware; `0` when microcompact is disabled. Read by the tinyagents
    /// turn to configure `MicrocompactMiddleware`.
    microcompact_keep_recent: usize,
    /// When `true`, the tinyagents turn installs the LLM summarization step
    /// (`ContextCompressionMiddleware`). Gated by both `[context].enabled` and
    /// `[context].autocompact_enabled` so a diagnostic/test opt-out doesn't spend
    /// summarizer tokens or rewrite history. See [`ContextConfig::autocompact_enabled`].
    autocompact_enabled: bool,
    /// `[context].compaction_trigger_tokens` and `compaction_strategy`. See
    /// [`ContextConfig::compaction_settings`].
    compaction: crate::config::CompactionSettings,
}

impl ContextManager {
    /// Construct a manager for a session.
    ///
    /// * `config` — the loaded [`ContextConfig`] section.
    /// * `default_prompt_builder` — the builder [`build_system_prompt`]
    ///   calls. For most agents this is `SystemPromptBuilder::with_defaults()`.
    ///
    /// The manager no longer owns a summarizer: live history reduction moved
    /// to the tinyagents graph (issue #4249). What remains here is the system
    /// prompt, the stats/utilisation surface and tool-result budgeting.
    pub fn new(config: &ContextConfig, default_prompt_builder: SystemPromptBuilder) -> Self {
        Self {
            stats_state: ContextStatsState::new(),
            default_prompt_builder,
            enabled: config.enabled,
            tool_result_budget_bytes: config.tool_result_budget_bytes,
            prefer_markdown_tool_output: config.prefer_markdown_tool_output,
            compaction_enabled: config.compaction_enabled,
            microcompact_keep_recent: if config.microcompact_enabled {
                config.microcompact_keep_recent
            } else {
                0
            },
            // Summarization is off when the whole context system is disabled OR
            // autocompaction specifically is turned off.
            autocompact_enabled: config.enabled && config.autocompact_enabled,
            compaction: config.compaction_settings(),
        }
    }

    /// Whether the agent loop should ask tools to render their output as
    /// markdown (when supported) instead of JSON, to save LLM tokens.
    pub fn prefer_markdown_tool_output(&self) -> bool {
        self.prefer_markdown_tool_output
    }

    /// Number of most-recent tool results the microcompact middleware keeps
    /// verbatim; `0` when microcompact is disabled. Read by the tinyagents turn
    /// to configure `MicrocompactMiddleware`.
    pub fn microcompact_keep_recent(&self) -> usize {
        self.microcompact_keep_recent
    }

    /// Byte budget for an individual tool result before the TinyAgents
    /// tool-output middleware cap fires.
    pub fn tool_result_budget_bytes(&self) -> usize {
        self.tool_result_budget_bytes
    }

    /// Whether native tool-output compaction (Stage 1a) is enabled. Agents
    /// read this when a tool returns to decide whether to content-aware
    /// compress the result before the byte cap and before it enters history.
    pub fn compaction_enabled(&self) -> bool {
        self.compaction_enabled
    }

    /// Whether the tinyagents turn should install the LLM summarization step.
    /// `false` when `[context].enabled = false` or `autocompact_enabled = false`
    /// — the diagnostic/test opt-outs the legacy reducer honored before
    /// requesting autocompaction. Read by the chat turn when building
    /// `TurnContextMiddleware`.
    pub fn autocompact_enabled(&self) -> bool {
        self.autocompact_enabled
    }

    /// The compaction trigger override (benchmarks / debugging) and the
    /// checkpoint strategy.
    pub fn compaction(&self) -> crate::config::CompactionSettings {
        self.compaction
    }

    // ─── Budget tracking ──────────────────────────────────────────

    /// Feed the latest provider [`BilledUsage`] into utilisation stats.
    pub fn record_usage(&mut self, usage: &BilledUsage) {
        self.stats_state.record_usage(usage);
    }

    // ─── Prompt building ───────────────────────────────────────────

    /// Assemble the opening system prompt for a session using the
    /// manager's default [`SystemPromptBuilder`].
    ///
    /// The returned bytes are the full system prompt, intended to be built
    /// once at session start and reused verbatim on every turn. Callers that
    /// can carry cache breakpoints to the provider should use
    /// [`Self::build_system_prompt_tiered`] instead; this wrapper exists for
    /// the call sites that only want the bytes.
    pub fn build_system_prompt(&self, ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.build_system_prompt_tiered(ctx)?.text)
    }

    /// Assemble the system prompt and report its cache-tier boundaries.
    ///
    /// The doc comment above used to end "the inference backend's prefix cache
    /// picks up the stable prefix automatically, so no boundary marker is
    /// emitted." That is true of backends with automatic longest-prefix caching
    /// and false of Anthropic, which caches nothing without an explicit
    /// breakpoint — so on Anthropic-family models this codebase re-paid full
    /// input price on a ~37k-token prefix, every turn, silently.
    pub fn build_system_prompt_tiered(
        &self,
        ctx: &PromptContext<'_>,
    ) -> Result<crate::agent::prompts::TieredPrompt> {
        self.default_prompt_builder.build_tiered(ctx)
    }

    // ─── Observability ─────────────────────────────────────────────

    /// Read-only snapshot of the current budget state.
    pub fn stats(&self) -> ContextStats {
        let utilisation_pct = self.stats_state.utilization_pct();
        ContextStats {
            utilisation_pct,
            input_tokens: self.stats_state.last_input_tokens(),
            output_tokens: self.stats_state.last_output_tokens(),
            context_window: self.stats_state.context_window(),
        }
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
