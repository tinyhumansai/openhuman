//! `Agent` and `AgentBuilder` struct definitions.
//!
//! The data shapes live here, separate from their behaviour, so the
//! rest of the sub-module (`builder.rs`, `turn.rs`, `runtime.rs`) can
//! focus on logic. Fields are `pub(super)` so sibling files that
//! `impl Agent`/`impl AgentBuilder` can see them without the whole
//! crate gaining field access.

use crate::openhuman::agent::context_pipeline::ContextPipeline;
use crate::openhuman::agent::dispatcher::ToolDispatcher;
use crate::openhuman::agent::hooks::PostTurnHook;
use crate::openhuman::agent::memory_loader::MemoryLoader;
use crate::openhuman::agent::prompt::SystemPromptBuilder;
use crate::openhuman::memory::Memory;
use crate::openhuman::providers::{ConversationMessage, Provider};
use crate::openhuman::tools::{Tool, ToolSpec};
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
    pub(super) prompt_builder: SystemPromptBuilder,
    pub(super) tool_dispatcher: Box<dyn ToolDispatcher>,
    pub(super) memory_loader: Box<dyn MemoryLoader>,
    pub(super) config: crate::openhuman::config::AgentConfig,
    pub(super) model_name: String,
    pub(super) temperature: f64,
    pub(super) workspace_dir: std::path::PathBuf,
    pub(super) identity_config: crate::openhuman::config::IdentityConfig,
    pub(super) skills: Vec<crate::openhuman::skills::Skill>,
    pub(super) auto_save: bool,
    /// Last memory context loaded for the current turn. Stored so it can
    /// be forwarded to subagents via `ParentExecutionContext`.
    pub(super) last_memory_context: Option<String>,
    pub(super) history: Vec<ConversationMessage>,
    pub(super) classification_config: crate::openhuman::config::QueryClassificationConfig,
    pub(super) available_hints: Vec<String>,
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) learning_enabled: bool,
    pub(super) event_session_id: String,
    pub(super) event_channel: String,
    /// Layered context reduction pipeline (tool-result budget →
    /// microcompact → autocompact signal → session-memory extraction
    /// trigger). Owned by the agent so its state (token counters,
    /// session-memory extraction deltas, compaction circuit breaker)
    /// persists across turns. See
    /// [`crate::openhuman::agent::context_pipeline`] for the stage
    /// ordering and cache-safety contract.
    pub(super) context_pipeline: ContextPipeline,
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
    pub(super) model_name: Option<String>,
    pub(super) temperature: Option<f64>,
    pub(super) workspace_dir: Option<std::path::PathBuf>,
    pub(super) identity_config: Option<crate::openhuman::config::IdentityConfig>,
    pub(super) skills: Option<Vec<crate::openhuman::skills::Skill>>,
    pub(super) auto_save: Option<bool>,
    pub(super) classification_config: Option<crate::openhuman::config::QueryClassificationConfig>,
    pub(super) available_hints: Option<Vec<String>>,
    pub(super) post_turn_hooks: Vec<Arc<dyn PostTurnHook>>,
    pub(super) learning_enabled: bool,
    pub(super) event_session_id: Option<String>,
    pub(super) event_channel: Option<String>,
}

impl Default for AgentBuilder {
    fn default() -> Self {
        Self::new()
    }
}
