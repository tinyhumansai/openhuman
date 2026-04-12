//! `Agent` and `AgentBuilder` struct definitions.
//!
//! The data shapes live here, separate from their behaviour, so the
//! rest of the sub-module (`builder.rs`, `turn.rs`, `runtime.rs`) can
//! focus on logic. Fields are `pub(super)` so sibling files that
//! `impl Agent`/`impl AgentBuilder` can see them without the whole
//! crate gaining field access.

use crate::openhuman::agent::dispatcher::ToolDispatcher;
use crate::openhuman::agent::hooks::PostTurnHook;
use crate::openhuman::agent::memory_loader::MemoryLoader;
use crate::openhuman::context::prompt::SystemPromptBuilder;
use crate::openhuman::context::ContextManager;
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ChatMessage, ConversationMessage, Provider};
use crate::openhuman::tools::{Tool, ToolSpec};
use std::path::PathBuf;
use std::sync::Arc;

/// An autonomous or semi-autonomous AI agent.
///
/// The `Agent` is the central component that manages conversation state,
/// executes tools based on model requests, and interacts with the memory
/// system to maintain context across turns.
pub struct Agent {
    pub(super) provider: Arc<dyn Provider>,
    /// Full tool registry. Sub-agents pull from this via
    /// [`ParentExecutionContext::all_tools`].
    pub(super) tools: Arc<Vec<Box<dyn Tool>>>,
    /// Full tool specs — sub-agents receive these via
    /// [`ParentExecutionContext::all_tool_specs`].
    pub(super) tool_specs: Arc<Vec<ToolSpec>>,
    /// Tool specs filtered by `visible_tool_names`. These are the specs
    /// actually sent to the provider in the main agent's chat requests.
    /// When `visible_tool_names` is empty this equals `tool_specs`.
    pub(super) visible_tool_specs: Arc<Vec<ToolSpec>>,
    /// When non-empty, only these tool names are visible in the main
    /// agent's prompt and callable by the main agent. Sub-agents ignore
    /// this filter — they apply per-definition whitelists in the runner.
    /// Empty = no filter (all tools visible, backward compat).
    pub(super) visible_tool_names: std::collections::HashSet<String>,
    pub(super) memory: Arc<dyn Memory>,
    pub(super) tool_dispatcher: Box<dyn ToolDispatcher>,
    pub(super) memory_loader: Box<dyn MemoryLoader>,
    pub(super) config: crate::openhuman::config::AgentConfig,
    pub(super) model_name: String,
    pub(super) temperature: f64,
    pub(super) workspace_dir: std::path::PathBuf,
    pub(super) skills: Vec<crate::openhuman::skills::Skill>,
    pub(super) auto_save: bool,
    /// Last memory context loaded for the current turn. Stored so it can
    /// be forwarded to subagents via `ParentExecutionContext`.
    pub(super) last_memory_context: Option<String>,
    /// Explicit cache boundary for the current rendered system prompt.
    /// `None` means the prompt is treated as entirely dynamic.
    pub(super) system_prompt_cache_boundary: Option<usize>,
    pub(super) history: Vec<ConversationMessage>,
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) learning_enabled: bool,
    pub(super) event_session_id: String,
    pub(super) event_channel: String,
    /// Human-readable agent definition name (e.g. `"main"`,
    /// `"code_executor"`). Used as the `{agent}` component in session
    /// transcript paths: `sessions/DDMMYYYY/{agent}_{index}.md`.
    pub(super) agent_definition_name: String,
    /// Resolved filesystem path for this session's transcript file.
    /// Set on first write, reused for subsequent overwrites within the
    /// same session.
    pub(super) session_transcript_path: Option<PathBuf>,
    /// Messages loaded from a previous session transcript on resume.
    /// Consumed once (via `.take()`) on the first turn to provide a
    /// byte-identical prefix for KV cache reuse.
    pub(super) cached_transcript_messages: Option<Vec<ChatMessage>>,
    /// Per-session [`ContextManager`] — owns the system-prompt
    /// builder, the layered reduction pipeline (tool-result budget →
    /// microcompact → autocompact signal → session-memory extraction
    /// trigger), the guard's compaction circuit breaker, and the LLM
    /// summarizer that runs when the pipeline asks for autocompaction.
    /// Constructed once at session start so its budget counters and
    /// session-memory deltas persist across turns. See
    /// [`crate::openhuman::context`] for the full surface.
    pub(super) context: ContextManager,
}

/// A builder for creating `Agent` instances with custom configuration.
pub struct AgentBuilder {
    pub(super) provider: Option<Arc<dyn Provider>>,
    pub(super) tools: Option<Vec<Box<dyn Tool>>>,
    /// When set, restricts which tools the main agent sees/calls.
    pub(super) visible_tool_names: Option<std::collections::HashSet<String>>,
    pub(super) memory: Option<Arc<dyn Memory>>,
    pub(super) prompt_builder: Option<SystemPromptBuilder>,
    pub(super) tool_dispatcher: Option<Box<dyn ToolDispatcher>>,
    pub(super) memory_loader: Option<Box<dyn MemoryLoader>>,
    pub(super) config: Option<crate::openhuman::config::AgentConfig>,
    /// Optional [`ContextConfig`] override threaded through from
    /// `Agent::from_config`. When unset the builder falls back to
    /// [`crate::openhuman::config::ContextConfig::default`].
    pub(super) context_config: Option<crate::openhuman::config::ContextConfig>,
    pub(super) model_name: Option<String>,
    pub(super) temperature: Option<f64>,
    pub(super) workspace_dir: Option<std::path::PathBuf>,
    pub(super) skills: Option<Vec<crate::openhuman::skills::Skill>>,
    pub(super) auto_save: Option<bool>,
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) learning_enabled: bool,
    pub(super) event_session_id: Option<String>,
    pub(super) event_channel: Option<String>,
    pub(super) agent_definition_name: Option<String>,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_builder_default_matches_new() {
        let builder = AgentBuilder::new();
        let default_builder = AgentBuilder::default();

        assert_eq!(builder.learning_enabled, default_builder.learning_enabled);
        assert_eq!(builder.auto_save, default_builder.auto_save);
        assert!(builder.provider.is_none());
        assert!(builder.tools.is_none());
        assert!(builder.memory.is_none());
        assert!(builder.event_session_id.is_none());
        assert!(builder.event_channel.is_none());
        assert!(builder.agent_definition_name.is_none());
        assert!(builder.post_turn_hooks.is_empty());
    }
}
