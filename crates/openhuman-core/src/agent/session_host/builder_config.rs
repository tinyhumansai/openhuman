//! Builder configuration state for constructing an OpenHuman session host.
//!
//! Kept separate from the live host/session state so the factory can consume
//! its inputs without making the runtime data shape harder to audit.

use crate::agent::prompts::SystemPromptBuilder;
use crate::agent::tinyagents::TurnModelSource;
use std::sync::Arc;
use tinyagents_harness::host::LearningSink;
use tinytools::Tool;
use tinytools_agent::dialect::ToolDialect;

/// A builder for creating `OpenHumanSessionHost` instances with custom configuration.
pub struct SessionHostBuilder {
    pub(super) permanent_tool_names: std::collections::HashSet<String>,
    pub(super) turn_model_source: Option<TurnModelSource>,
    /// Explicit host config for injected-model attachment staging and services.
    pub(super) runtime_config: Option<Arc<crate::config::Config>>,
    pub(super) tools: Option<Vec<Box<dyn Tool>>>,
    /// Host-owned handle shared with the durable `use_skill` tool.
    pub(super) pack_registry_handle: Option<tinyagents_harness_tools::skills::PackRegistryHandle>,
    /// Delegation tools synthesised for the session's initial connection set.
    /// Held in [`OpenHumanSessionHost::synthesized_tools`], never inside [`OpenHumanSessionHost::tools`].
    pub(super) synthesized_tools: Option<Vec<Box<dyn Tool>>>,
    /// When set, restricts which tools the main agent sees/calls.
    pub(super) visible_tool_names: Option<std::collections::HashSet<String>>,
    /// Names removed from the final provider-visible set after wildcard
    /// expansion and tool synthesis.
    pub(super) withheld_tool_names: std::collections::HashSet<String>,
    /// See [`SessionHostBuilder::deferred_tools`].
    pub(super) deferred_tools: Vec<String>,
    /// Optional explicit profile ceiling for tools delegated agents may inherit.
    /// Channel-policy restrictions are intersected during [`Self::build`].
    pub(super) subagent_tool_ceiling_names: Option<std::collections::HashSet<String>>,
    pub(super) prompt_builder: Option<SystemPromptBuilder>,
    pub(super) tool_dispatcher: Option<Box<dyn ToolDialect>>,
    pub(super) config: Option<crate::config::AgentConfig>,
    /// Optional [`ContextConfig`] override threaded through from
    /// `OpenHumanSessionHost::from_config`. When unset the builder falls back to
    /// [`crate::config::ContextConfig::default`].
    pub(super) context_config: Option<crate::config::ContextConfig>,
    pub(super) model_name: Option<String>,
    /// User vision flag for the resolved model; `None` → `false` in `build()`.
    pub(super) model_vision: Option<bool>,
    pub(super) temperature: Option<f64>,
    pub(super) workspace_dir: Option<std::path::PathBuf>,
    pub(super) action_dir: Option<std::path::PathBuf>,
    /// Optional turn workspace descriptor forwarded to [`OpenHumanSessionHost`] at build time.
    /// Defaults to `None` (shared `action_dir` cwd).
    pub(super) workspace_descriptor: Option<tinytools::WorkspaceDescriptor>,
    pub(super) workflows: Option<Vec<tinyskills::Skill>>,
    /// OpenHumanSessionHost workflows to surface in the prompt. Populated from `load_workflows`
    /// at session start; defaults to empty when not explicitly set.
    pub(super) post_turn_hooks: Vec<Arc<dyn LearningSink>>,
    pub(super) event_session_id: Option<String>,
    pub(super) event_channel: Option<String>,
    pub(super) agent_definition_name: Option<String>,
    /// The session's own definition, when the caller has one rather than a
    /// registry id to name.
    ///
    /// A hosted root invocation resolves its agent id against the host
    /// catalogue before it composes a message, and refuses the turn when the
    /// id is not there. `from_config_with_definition` stamps a caller's
    /// definition on the session for exactly that reason; a direct builder
    /// caller had no equivalent, so its only catalogue was the process-wide
    /// [`AgentDefinitionRegistry`] — read once from `<workspace>/agents/*.toml`
    /// at startup, with no refresh.
    ///
    /// Set here it becomes
    /// [`OpenHumanHostBase::session_definition`](crate::agent::tinyagents::host::OpenHumanHostBase::session_definition),
    /// which outranks the registry for this session's own id.
    ///
    /// `None` (the default) leaves every existing caller as it was.
    ///
    /// [`AgentDefinitionRegistry`]: crate::agent_registry::host_definition::AgentDefinitionRegistry
    pub(super) session_definition:
        Option<Arc<crate::agent_registry::host_definition::AgentDefinition>>,
    /// Directory chain of parent session keys for a sub-agent. `None`
    /// (default) means this is a root session — its transcript lands
    /// flat in `session_raw/DDMMYYYY/{session_key}.jsonl`. Populated
    /// by the sub-agent runner so nested delegations produce a tree.
    pub(super) session_parent_prefix: Option<String>,
    /// Forwarded to [`OpenHumanSessionHost::session_history_locator`]. `None` (default) means
    /// real files; set it with
    /// [`with_session_history_locator`][super::builder::SessionHostBuilder::with_session_history_locator]
    /// to substitute the transcript backing store for the whole turn path.
    pub(super) session_history_locator:
        Option<std::sync::Arc<dyn tinyagents_session::transcript::TranscriptLocator>>,
    /// Forwarded to [`OpenHumanSessionHost::omit_memory_context`] at `build()`
    /// time; `None` falls back to `false` (inject `context.md`).
    pub(super) omit_memory_context: Option<bool>,
    /// Optional payload-summarizer threaded through to [`OpenHumanSessionHost`] at
    /// build time. Defaults to `None`; the orchestrator branch in
    /// [`super::builder::OpenHumanSessionHost::build_session_agent_inner`] sets this
    /// to a `SubagentPayloadSummarizer` instance.
    pub(super) payload_summarizer:
        Option<Arc<crate::agent::tinyagents::payload_summarizer::DynPayloadSummarizer>>,
    /// Per-agent TokenJuice tool-output compression profile.
    pub(super) tokenjuice_compression: tinyjuice_bus::types::AgentTokenjuiceCompression,
    /// Optional pre-execution tool policy. Defaults to allow-all.
    pub(super) tool_policy: Option<
        Arc<
            dyn tinyagents_harness::middleware::ToolCallPolicy<
                crate::agent::tinyagents::host::OpenHumanRunContext,
            >,
        >,
    >,
}

impl Default for SessionHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}
