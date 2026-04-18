//! Global context management for agent sessions.
//!
//! This module is the single home for everything that shapes what an LLM
//! sees during a conversation:
//!
//! 1. **System prompt assembly** — [`prompt::SystemPromptBuilder`] and its
//!    composable [`prompt::PromptSection`] trait. Main agents, sub-agents,
//!    and channels all build their opening system prompts through this
//!    module; there is no parallel implementation elsewhere in the crate.
//!
//! 2. **Mechanical history reduction** — the layered [`pipeline`] (tool
//!    result budget → trim → microcompact → autocompact signal → session
//!    memory trigger) keeps the in-flight conversation within the
//!    provider's context window.
//!
//! 3. **Summarization execution** — when the pipeline asks for
//!    autocompaction, [`ContextManager`] dispatches the LLM summarization
//!    call via a [`summarizer::Summarizer`] implementation. Agents do not
//!    call the provider directly for compaction; they hand their history
//!    to the manager and get back a reduced history.
//!
//! Agents hold a single [`ContextManager`] per session. The manager owns
//! per-conversation state (budget, circuit breaker, session-memory
//! counters) but all of the shared logic — prompt sections, reduction
//! stages, the summarizer contract — lives in this module so new agent
//! archetypes and delegation tools do not need to re-wire any of it.
//!
//! Submodules are added incrementally as the `agent/` → `context/`
//! migration lands (see plan `misty-bubbling-bunny.md`).

pub mod channels_prompt;
pub mod debug_dump;
pub mod guard;
pub mod manager;
pub mod microcompact;
pub mod pipeline;
pub mod prompt;
pub mod session_memory;
pub mod summarizer;
pub mod tool_result_budget;

pub use guard::{ContextCheckResult, ContextGuard};
pub use manager::{ContextManager, ContextStats, ReductionOutcome};
pub use microcompact::{
    microcompact, MicrocompactStats, CLEARED_PLACEHOLDER, DEFAULT_KEEP_RECENT_TOOL_RESULTS,
};
pub use pipeline::{ContextPipeline, ContextPipelineConfig, PipelineOutcome};
pub use prompt::{
    ArchetypePromptSection, DateTimeSection, IdentitySection, LearnedContextData, PromptContext,
    PromptSection, PromptTool, RuntimeSection, SafetySection, SystemPromptBuilder, ToolsSection,
    WorkspaceSection,
};
pub use session_memory::{
    SessionMemoryConfig, SessionMemoryState, ARCHIVIST_EXTRACTION_PROMPT, DEFAULT_MIN_TOKEN_GROWTH,
    DEFAULT_MIN_TOOL_CALLS, DEFAULT_MIN_TURNS_BETWEEN,
};
pub use summarizer::{ProviderSummarizer, Summarizer, SummaryStats};
pub use tool_result_budget::{
    apply_tool_result_budget, BudgetOutcome, DEFAULT_TOOL_RESULT_BUDGET_BYTES,
};
