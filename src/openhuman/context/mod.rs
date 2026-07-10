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
//! 2. **Mechanical history bookkeeping** — [`stats::ContextStatsState`] records
//!    provider usage and session-memory triggers. Live reduction runs in the
//!    TinyAgents middleware stack.
//!
//! Agents hold a single [`ContextManager`] per session. The manager owns
//! per-conversation state (budget, utilisation, session-memory counters)
//! while prompt assembly remains centralized here.
//!
//! Submodules are added incrementally as the `agent/` → `context/`
//! migration lands (see plan `misty-bubbling-bunny.md`).

pub mod channels_prompt;
pub mod manager;
pub mod prompt;
pub mod session_memory;
pub mod stats;

pub use manager::{ContextManager, ContextStats};
pub use prompt::{
    ArchetypePromptSection, DateTimeSection, IdentitySection, LearnedContextData, PromptContext,
    PromptSection, PromptTool, RuntimeSection, SafetySection, SystemPromptBuilder, ToolsSection,
    WorkspaceSection,
};
pub use session_memory::{
    SessionMemoryConfig, SessionMemoryState, ARCHIVIST_EXTRACTION_PROMPT, DEFAULT_MIN_TOKEN_GROWTH,
    DEFAULT_MIN_TOOL_CALLS, DEFAULT_MIN_TURNS_BETWEEN,
};
/// Default per-tool-result budget. The live TinyAgents tool-output middleware
/// and action-workspace artifact previews enforce this outside the legacy
/// context reducer modules.
pub const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;

/// Placeholder used by the TinyAgents microcompact middleware when it clears
/// older tool-result bodies.
pub const CLEARED_PLACEHOLDER: &str = "[Old tool result content cleared]";

/// Default number of most-recent tool-result envelopes the TinyAgents
/// microcompact middleware leaves intact.
pub const DEFAULT_KEEP_RECENT_TOOL_RESULTS: usize = 5;
