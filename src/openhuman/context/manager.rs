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
//!    stats, tool-result budget config, and session-memory trigger state.
//!    Live history reduction/summarization moved to the
//!    tinyagents graph (`ContextCompressionMiddleware` +
//!    `MessageTrimMiddleware`, issue #4249); this manager no longer runs
//!    an in-turn summarizer.
//!
//! # What it doesn't own
//!
//! The session-memory extraction *task itself* still lives in the
//! agent harness (`turn.rs` spawns the archivist sub-agent). The
//! manager only owns the *state* that decides whether the trigger
//! should fire; it exposes that via
//! [`ContextManager::should_extract_session_memory`] so `turn.rs` can
//! gate its existing `spawn_subagent` call.

use super::prompt::{PromptContext, SystemPromptBuilder};
use super::session_memory::SessionMemoryConfig;
use super::stats::{ContextStatsState, SessionMemoryHandle};
use crate::openhuman::config::ContextConfig;
use crate::openhuman::inference::provider::UsageInfo;
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
    pub session_memory_total_tokens: u64,
    pub session_memory_current_turn: u64,
    pub session_memory_total_tool_calls: u64,
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
    /// `Agent::execute_tool_call` before the byte cap. On by default; the
    /// kill-switch lives here so every caller reads one source of truth.
    /// See [`ContextConfig::compaction_enabled`].
    compaction_enabled: bool,
    /// Number of most-recent tool results kept verbatim by the microcompact
    /// middleware; `0` when microcompact is disabled. Read by the tinyagents
    /// turn to configure `MicrocompactMiddleware`.
    microcompact_keep_recent: usize,
    /// When `true`, the harness runs a mandatory first-turn context
    /// collection pass before the orchestrator LLM runs. Read once at
    /// session construction so it only affects newly started threads.
    /// See [`ContextConfig::super_context_enabled`].
    super_context_enabled: bool,
    /// When `true`, the tinyagents turn installs the LLM summarization step
    /// (`ContextCompressionMiddleware`). Gated by both `[context].enabled` and
    /// `[context].autocompact_enabled` so a diagnostic/test opt-out doesn't spend
    /// summarizer tokens or rewrite history. See [`ContextConfig::autocompact_enabled`].
    autocompact_enabled: bool,
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
    /// prompt, the stats/utilisation surface, tool-result budgeting, and
    /// session-memory bookkeeping.
    pub fn new(config: &ContextConfig, default_prompt_builder: SystemPromptBuilder) -> Self {
        Self {
            stats_state: ContextStatsState::new(SessionMemoryConfig {
                min_token_growth: config.session_memory.min_token_growth,
                min_tool_calls: config.session_memory.min_tool_calls,
                min_turns_between: config.session_memory.min_turns_between,
            }),
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
            super_context_enabled: config.super_context_enabled,
            // Summarization is off when the whole context system is disabled OR
            // autocompaction specifically is turned off.
            autocompact_enabled: config.enabled && config.autocompact_enabled,
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

    /// Whether "super context" is enabled — i.e. whether the harness
    /// should run a mandatory read-only context-collection pass on the
    /// first turn of a new thread before the orchestrator LLM runs.
    /// Read by `Agent::turn`. See [`ContextConfig::super_context_enabled`].
    pub fn super_context_enabled(&self) -> bool {
        self.super_context_enabled
    }

    /// Whether the tinyagents turn should install the LLM summarization step.
    /// `false` when `[context].enabled = false` or `autocompact_enabled = false`
    /// — the diagnostic/test opt-outs the legacy reducer honored before
    /// requesting autocompaction. Read by the chat turn when building
    /// `TurnContextMiddleware`.
    pub fn autocompact_enabled(&self) -> bool {
        self.autocompact_enabled
    }

    /// Force-disable the first-turn super-context pass for this session,
    /// regardless of the config default. Used by non-interactive orchestrator
    /// builds (e.g. read-only model-council jurors) where a scout pass would add
    /// an unexpected LLM call and perturb deterministic call sequences.
    pub fn set_super_context_enabled(&mut self, enabled: bool) {
        self.super_context_enabled = enabled;
    }

    // ─── Budget tracking ──────────────────────────────────────────

    /// Feed the latest provider [`UsageInfo`] into utilisation stats and the
    /// session-memory state.
    pub fn record_usage(&mut self, usage: &UsageInfo) {
        self.stats_state.record_usage(usage);
    }

    /// Bump the session-memory turn counter (called once per user turn).
    pub fn tick_turn(&mut self) {
        self.stats_state.tick_turn();
    }

    /// Accumulate a turn's tool-call count into the session-memory state.
    pub fn record_tool_calls(&mut self, n: usize) {
        self.stats_state.record_tool_calls(n);
    }

    /// Whether the caller should spawn a background session-memory
    /// extraction this turn. Delegates to the underlying stats state; the
    /// manager does not spawn the extraction itself.
    pub fn should_extract_session_memory(&self) -> bool {
        self.stats_state.should_extract_session_memory()
    }

    /// Mark a session-memory extraction as started (so repeated
    /// calls to [`should_extract_session_memory`] return `false` until
    /// the extraction completes).
    pub fn mark_session_memory_started(&mut self) {
        self.stats_state.mark_session_memory_started();
    }

    /// Mark a session-memory extraction as complete — resets deltas.
    pub fn mark_session_memory_complete(&mut self) {
        self.stats_state.mark_session_memory_complete();
    }

    /// Mark a session-memory extraction as failed — keeps deltas
    /// intact so the next turn retries.
    pub fn mark_session_memory_failed(&mut self) {
        self.stats_state.mark_session_memory_failed();
    }

    /// Clone the shared session-memory handle so a detached background
    /// task (see `turn.rs::spawn_session_memory_extraction`) can mark
    /// the extraction complete or failed once it finishes. The
    /// foreground path is expected to call
    /// [`Self::mark_session_memory_started`] *before* spawning so
    /// overlapping turns don't fire duplicate extractions while this
    /// one is in flight.
    pub(crate) fn session_memory_handle(&self) -> SessionMemoryHandle {
        self.stats_state.session_memory_handle()
    }

    // ─── Prompt building ───────────────────────────────────────────

    /// Assemble the opening system prompt for a session using the
    /// manager's default [`SystemPromptBuilder`].
    ///
    /// The returned bytes are the full system prompt, intended to be
    /// built once at session start and reused verbatim on every turn —
    /// the inference backend's prefix cache picks up the stable prefix
    /// automatically, so no boundary marker is emitted.
    pub fn build_system_prompt(&self, ctx: &PromptContext<'_>) -> Result<String> {
        let prompt = self.default_prompt_builder.build(ctx)?;
        Ok(prompt)
    }

    /// Assemble the system prompt via a caller-supplied builder.
    ///
    /// Sub-agents pass `SystemPromptBuilder::for_subagent(...)` and
    /// channels pass `with_defaults()` chained with a
    /// `ChannelCapabilitiesSection`. Either way the builder itself
    /// lives in [`super::prompt`] — no caller needs to know how
    /// sections are composed internally.
    pub fn build_system_prompt_with(
        &self,
        builder: &SystemPromptBuilder,
        ctx: &PromptContext<'_>,
    ) -> Result<String> {
        let prompt = builder.build(ctx)?;
        Ok(prompt)
    }

    // ─── Observability ─────────────────────────────────────────────

    /// Read-only snapshot of the current budget state.
    pub fn stats(&self) -> ContextStats {
        let utilisation_pct = self.stats_state.utilization_pct();
        let sm = self.stats_state.session_memory_snapshot();
        ContextStats {
            utilisation_pct,
            input_tokens: self.stats_state.last_input_tokens(),
            output_tokens: self.stats_state.last_output_tokens(),
            context_window: self.stats_state.context_window(),
            session_memory_total_tokens: sm.total_tokens,
            session_memory_current_turn: sm.current_turn,
            session_memory_total_tool_calls: sm.total_tool_calls,
        }
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod tests;
