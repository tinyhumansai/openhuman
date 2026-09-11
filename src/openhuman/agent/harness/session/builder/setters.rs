//! `AgentBuilder` fluent setters. See `builder_build.rs` for the `build()`
//! validator that assembles the final `Agent`.

use crate::openhuman::agent::harness::session::types::AgentBuilder;
use crate::openhuman::agent::harness::TriggerMemoryAgent;
use crate::openhuman::config::ContextConfig;
use crate::openhuman::memory::Memory;
use crate::openhuman::tools::Tool;
use std::sync::Arc;

impl AgentBuilder {
    /// Creates a new `AgentBuilder` with default values.
    pub fn new() -> Self {
        Self {
            turn_model_source: None,
            tools: None,
            synthesized_tools: None,
            visible_tool_names: None,
            subagent_tool_ceiling_names: None,
            memory: None,
            shared_experience_memory: None,
            auto_recall: None,
            prompt_builder: None,
            tool_dispatcher: None,
            config: None,
            context_config: None,
            model_name: None,
            model_vision: None,
            temperature: None,
            workspace_dir: None,
            action_dir: None,
            workspace_descriptor: None,
            workflows: None,
            auto_save: None,
            post_turn_hooks: Vec::new(),
            learning_enabled: false,
            explicit_preferences_enabled: true,
            event_session_id: None,
            event_channel: None,
            agent_definition_name: None,
            active_profile_id: None,
            personality_soul_md: None,
            personality_memory_md: None,
            memory_subdir: None,
            session_raw_subdir: None,
            session_parent_prefix: None,
            session_history_locator: None,
            omit_profile: None,
            omit_memory_md: None,
            payload_summarizer: None,
            trigger_memory_agent: None,
            tokenjuice_compression:
                crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Full,
            tool_policy: None,
            archivist_hook: None,
        }
    }

    /// Sets an already-constructed TinyAgents chat model. This is the native
    /// injection seam for tests and embedders; no legacy `Provider` adapter is
    /// constructed.
    pub fn chat_model(mut self, model: Arc<dyn tinyinference::model::ChatModel<()>>) -> Self {
        self.turn_model_source =
            Some(crate::openhuman::agent::tinyagents::TurnModelSource::from_model(model));
        self
    }

    /// Sets the AI provider as a **crate-native** turn-model source (Phase 3 P3-B):
    /// `build`/`build_summarizer` construct crate `ChatModel`s from `(role, config)`
    /// via `create_turn_chat_model` (managed → `OpenHumanBackendModel`, local/cloud →
    /// crate `OpenAiModel`) instead of wrapping `provider` in `native model adapters.
    /// Used by the production session factory; the plain
    /// [`provider`](Self::provider) setter (Provider path) stays for tests that
    /// inject a mock they observe.
    pub fn crate_native_provider(
        mut self,
        role: impl Into<String>,
        config: Arc<crate::openhuman::config::Config>,
    ) -> Self {
        self.turn_model_source = Some(
            crate::openhuman::agent::tinyagents::TurnModelSource::new_crate_native(role, config),
        );
        self
    }

    /// Sets the available tools for the agent.
    pub fn tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
        self.tools = Some(tools);
        self
    }

    /// Sets the delegation tools synthesised for the session's initial
    /// connection set — see [`Agent::synthesized_tools`]. A name a durable
    /// tool already owns is dropped in [`Self::build`]. Defaults to none.
    pub fn synthesized_tools(mut self, tools: Vec<Box<dyn Tool>>) -> Self {
        self.synthesized_tools = Some(tools);
        self
    }

    /// Restricts which tools the main agent can see and call directly.
    /// Tools not in this set are still available to sub-agents via the
    /// runner. Pass `None` (default) to make all tools visible.
    pub fn visible_tool_names(mut self, names: std::collections::HashSet<String>) -> Self {
        self.visible_tool_names = Some(names);
        self
    }

    /// Restrict the tool names that delegated agents may inherit from the full
    /// registry. Empty/`None` keeps delegation governed by each child
    /// definition unless the channel policy adds a ceiling.
    pub fn subagent_tool_ceiling_names(mut self, names: std::collections::HashSet<String>) -> Self {
        self.subagent_tool_ceiling_names = Some(names);
        self
    }

    /// Sets the memory system for the agent.
    pub fn memory(mut self, memory: Arc<dyn Memory>) -> Self {
        self.memory = Some(memory);
        self
    }

    /// Retains the shared store for experience recall when `memory` is a
    /// dedicated profile subtree.
    pub fn shared_experience_memory(mut self, memory: Option<Arc<dyn Memory>>) -> Self {
        self.shared_experience_memory = memory;
        self
    }

    /// Binds Lane C, the gated pre-turn auto-recall of facts about the user
    /// (#6040). `None` leaves the lane out of the turn entirely.
    pub fn auto_recall(
        mut self,
        auto_recall: Option<Arc<crate::openhuman::memory::auto_recall::AutoRecall>>,
    ) -> Self {
        self.auto_recall = auto_recall;
        self
    }

    /// Sets the system prompt builder for the agent.
    pub fn prompt_builder(
        mut self,
        prompt_builder: crate::openhuman::agent::context::prompt::SystemPromptBuilder,
    ) -> Self {
        self.prompt_builder = Some(prompt_builder);
        self
    }

    /// Sets the tool dispatcher for the agent.
    pub fn tool_dispatcher(
        mut self,
        tool_dispatcher: Box<dyn crate::openhuman::agent::dispatcher::ToolDispatcher>,
    ) -> Self {
        self.tool_dispatcher = Some(tool_dispatcher);
        self
    }

    /// Sets the agent configuration.
    pub fn config(mut self, config: crate::openhuman::config::AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Sets the global context-management configuration. Threaded
    /// into the [`ContextManager`] constructed in [`Self::build`]. If
    /// not set the manager is constructed with
    /// [`ContextConfig::default`].
    pub fn context_config(mut self, context_config: ContextConfig) -> Self {
        self.context_config = Some(context_config);
        self
    }

    /// Sets the model name to use for chat requests.
    pub fn model_name(mut self, model_name: String) -> Self {
        self.model_name = Some(model_name);
        self
    }

    /// Sets the user-configured vision capability for the resolved model.
    /// Surfaced to the turn engine's image gate via the `current_model_vision`
    /// task-local. Defaults to `false` when unset.
    pub fn model_vision(mut self, model_vision: bool) -> Self {
        self.model_vision = Some(model_vision);
        self
    }

    /// Sets the temperature for chat requests.
    pub fn temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    /// Sets the workspace directory for the agent.
    pub fn workspace_dir(mut self, workspace_dir: std::path::PathBuf) -> Self {
        self.workspace_dir = Some(workspace_dir);
        self
    }

    pub fn action_dir(mut self, action_dir: std::path::PathBuf) -> Self {
        self.action_dir = Some(action_dir);
        self
    }

    /// Sets the per-profile workspace descriptor (section D of agent-profile
    /// homes). When set, the top-level chat turn threads it through so acting
    /// tools resolve their default cwd to the profile's dedicated workspace.
    pub fn workspace_descriptor(
        mut self,
        descriptor: Option<tinyagents_harness::workspace::WorkspaceDescriptor>,
    ) -> Self {
        self.workspace_descriptor = descriptor;
        self
    }

    /// Sets the active agent-profile id for this session (1a plumbing).
    ///
    /// `None` (default) is the profile-less session. When set, the id is
    /// carried on the built [`Agent`] and threaded into the post-turn
    /// [`TurnContext`](crate::openhuman::agent::hooks::TurnContext) so
    /// profile-scoped hooks (agent-experience capture) can stamp records with
    /// it. A `None` here keeps every downstream consumer on its legacy path.
    pub fn active_profile_id(mut self, profile_id: Option<String>) -> Self {
        self.active_profile_id = profile_id;
        self
    }

    /// Binds the active profile's SOUL.md as the session identity override.
    pub fn personality_soul_md(mut self, soul_md: Option<String>) -> Self {
        self.personality_soul_md = soul_md;
        self
    }

    /// Binds the active profile's curated MEMORY.md to the frozen session
    /// prompt. `None` keeps the legacy workspace-root fallback.
    pub fn personality_memory_md(mut self, memory_md: Option<String>) -> Self {
        self.personality_memory_md = memory_md;
        self
    }

    pub fn profile_memory_storage(
        mut self,
        memory_subdir: String,
        session_raw_subdir: String,
    ) -> Self {
        self.memory_subdir = Some(memory_subdir);
        self.session_raw_subdir = Some(session_raw_subdir);
        self
    }

    /// Sets the skills available to the agent.
    pub fn workflows(mut self, skills: Vec<crate::openhuman::skills::Workflow>) -> Self {
        self.workflows = Some(skills);
        self
    }

    /// Enables or disables automatic saving of conversation history to memory.
    pub fn auto_save(mut self, auto_save: bool) -> Self {
        self.auto_save = Some(auto_save);
        self
    }

    /// Sets the post-turn hooks to be executed after each turn.
    pub fn post_turn_hooks(
        mut self,
        hooks: Vec<Arc<dyn crate::openhuman::agent::hooks::PostTurnHook>>,
    ) -> Self {
        self.post_turn_hooks = hooks;
        self
    }

    /// Enables or disables learning features.
    pub fn learning_enabled(mut self, enabled: bool) -> Self {
        self.learning_enabled = enabled;
        self
    }

    /// Enables or disables explicit-preference injection.
    ///
    /// When `true` (the default), preferences stored via `remember_preference`
    /// are fetched from the `user_profile` namespace and injected into the
    /// system prompt on every turn, independent of `learning_enabled`.
    pub fn explicit_preferences_enabled(mut self, enabled: bool) -> Self {
        self.explicit_preferences_enabled = enabled;
        self
    }

    /// Sets the event-bus `session_id` and `channel` used to tag
    /// `DomainEvent`s emitted by this agent.
    ///
    /// - `session_id` groups all events for a single user / conversation so
    ///   downstream subscribers can correlate turns, tool calls, and errors.
    /// - `channel` labels the source or stream the events originated from
    ///   (e.g. `"cli"`, `"telegram"`, `"rpc"`) — useful when multiple front
    ///   ends share the same subscriber pipeline.
    ///
    /// Both parameters are converted into owned `String`s and stored in
    /// `event_session_id` / `event_channel` respectively.
    pub fn event_context(
        mut self,
        session_id: impl Into<String>,
        channel: impl Into<String>,
    ) -> Self {
        self.event_session_id = Some(session_id.into());
        self.event_channel = Some(channel.into());
        self
    }

    /// Sets the agent definition id this session is running
    /// (`welcome`, `orchestrator`, `integrations_agent`, …).
    ///
    /// This value is stamped onto the built [`Agent`] and surfaces in
    /// the following places:
    ///
    /// * **Transcript filename on disk** — `transcript::write_transcript`
    ///   and `transcript::find_latest_transcript` use it as the
    ///   `{agent}` prefix in `sessions/DDMMYYYY/{agent}_{index}.md`.
    ///   Both the write path and the resume-lookup path read the same
    ///   field on `self`, so a session is always self-consistent; the
    ///   user-visible signal is which filename the transcript lands
    ///   under. Leaving it at the legacy `"main"` fallback silently
    ///   misfiles every non-orchestrator session under `main_*.md`.
    /// * **Transcript metadata header** — `transcript::write_transcript`
    ///   stamps it into the `<!-- session_transcript\nagent: {name}\n… -->`
    ///   block at the top of every `.md` file. This is the ground-truth
    ///   signal for "which agent definition ran this session" when
    ///   inspecting transcripts after the fact.
    /// * **[`PromptContext::agent_id`]** at prompt-build time (see
    ///   `turn.rs`). Today only one prompt section reads this field —
    ///   the `Connected Integrations` branch in `context/prompt.rs`
    ///   that special-cases `integrations_agent` vs every other agent — so
    ///   the current user-visible impact of a wrong id is limited to
    ///   the two bullets above. The stamped `prompt_builder` injected
    ///   by [`Agent::from_config_for_agent`] is what actually drives
    ///   prompt flavour per archetype, independent of this field. That
    ///   said, any future prompt section that branches on a
    ///   non-`integrations_agent` id (e.g. welcome-specific banner, planner-
    ///   specific rubric) would silently never fire if the field were
    ///   left at `"main"`, so keeping it correctly stamped closes a
    ///   latent foot-gun for code that hasn't been written yet.
    ///
    /// Callers building via [`Agent::from_config_for_agent`] get this
    /// wired automatically inside `build_session_agent_inner`; direct
    /// builder users (tests, CLI) must set it explicitly if they care
    /// about any of the surfaces above.
    pub fn agent_definition_name(mut self, name: impl Into<String>) -> Self {
        self.agent_definition_name = Some(name.into());
        self
    }

    /// Set the parent session-key chain for a sub-agent. Passing
    /// `Some("1713000000_orchestrator")` produces a sub-agent whose
    /// transcript filename is prefixed with the parent's session key,
    /// yielding a flat hierarchy on disk
    /// (`session_raw/DDMMYYYY/{parent}__{child}.jsonl`). Nested
    /// delegations chain further prefixes with `__`. Leave `None`
    /// (default) for root sessions.
    pub fn session_parent_prefix(mut self, prefix: Option<String>) -> Self {
        self.session_parent_prefix = prefix;
        self
    }

    /// Substitute the transcript backing store for this session.
    ///
    /// The one injection point for the S4 seam: the locator resolves both
    /// resume reads (`latest_for_agent` / `root_for_thread`) **and** binds the
    /// session's write handle (`open_stem`), so a fake supplied here takes the
    /// whole turn path off the filesystem. Leave unset in production — `None`
    /// resolves lazily to a
    /// [`FileTranscriptLocator`][super::super::transcript_history::FileTranscriptLocator]
    /// over the agent's current workspace, which is behaviourally identical to
    /// the pre-S4 free-function calls.
    pub(crate) fn with_session_history_locator(
        mut self,
        locator: std::sync::Arc<dyn super::super::transcript_history::SessionHistoryLocator>,
    ) -> Self {
        self.session_history_locator = Some(locator);
        self
    }

    /// Forward the target agent definition's `omit_profile` flag so
    /// [`Agent::build_system_prompt`] can decide whether to inject
    /// `PROFILE.md`. Only opt-in agents (welcome, orchestrator, the
    /// trigger pair) should set this to `false`.
    pub fn omit_profile(mut self, omit: bool) -> Self {
        self.omit_profile = Some(omit);
        self
    }

    /// Forward the target agent definition's `omit_memory_md` flag so
    /// [`Agent::build_system_prompt`] can decide whether to inject
    /// `MEMORY.md`. Same opt-in set as `omit_profile`.
    pub fn omit_memory_md(mut self, omit: bool) -> Self {
        self.omit_memory_md = Some(omit);
        self
    }

    /// Wire an oversized-tool-result summarizer into the agent. The live
    /// TinyAgents turn path passes it to `ToolOutputMiddleware`, which calls
    /// [`crate::openhuman::agent::tinyagents::payload_summarizer::PayloadSummarizer::maybe_summarize_in_parent`]
    /// on successful tool output and replaces the raw payload with the
    /// compressed summary on success. Currently set only for the orchestrator
    /// session by [`Agent::build_session_agent_inner`].
    pub fn payload_summarizer(
        mut self,
        summarizer: Arc<
            dyn crate::openhuman::agent::tinyagents::payload_summarizer::PayloadSummarizer,
        >,
    ) -> Self {
        self.payload_summarizer = Some(summarizer);
        self
    }

    /// Forward the target agent definition's pre-turn memory policy.
    pub fn trigger_memory_agent(mut self, policy: TriggerMemoryAgent) -> Self {
        self.trigger_memory_agent = Some(policy);
        self
    }

    /// Installs pre-execution policy middleware for tool calls.
    ///
    /// The default policy allows all calls. Custom policies can deny a call
    /// before `Tool::execute_with_options` runs.
    pub fn tool_policy(
        mut self,
        policy: Arc<dyn crate::openhuman::agent::tool_policy::ToolPolicy>,
    ) -> Self {
        self.tool_policy = Some(policy);
        self
    }

    /// Attach the production [`ArchivistHook`] instance so the session
    /// turn loop can call [`ArchivistHook::flush_open_segment`] at
    /// session-wind-down time, guaranteeing the trailing open segment is
    /// always finalized with an LLM recap + embedding.
    ///
    /// Set from `build_session_agent_inner` when
    /// `config.learning.episodic_capture_enabled` is `true` and a
    /// SQLite connection is available. Callers that construct an `Agent`
    /// directly (tests, CLI) can leave this `None` — flush is a no-op
    /// when the hook is absent.
    pub fn archivist_hook(
        mut self,
        hook: Option<Arc<crate::openhuman::agent::harness::archivist::ArchivistHook>>,
    ) -> Self {
        self.archivist_hook = hook;
        self
    }

    /// Set the per-agent TokenJuice tool-output compression profile.
    pub fn tokenjuice_compression(
        mut self,
        profile: crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression,
    ) -> Self {
        self.tokenjuice_compression = profile;
        self
    }
}
