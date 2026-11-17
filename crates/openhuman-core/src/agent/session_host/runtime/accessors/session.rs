use super::*;

impl OpenHumanSessionHost {
    /// The agent's working directory.
    pub fn workspace_dir(&self) -> &std::path::Path {
        &self.workspace_dir
    }

    /// The agent's currently-configured model name (before per-turn
    /// auto-classification).
    pub fn model_name(&self) -> &str {
        &self.model_name
    }

    /// Override the base model this session runs its top-level turns on. Set
    /// once before running: per-turn classification is disabled (the main agent
    /// is pinned to its configured model for KV-cache stability — see the model
    /// pin in `turn/core.rs`), so this sticks for the session and is not flipped
    /// mid-conversation. The realtime voice harness uses it to pin a fast,
    /// non-thinking model within the provider's response-time ceiling.
    pub fn set_model_name(&mut self, model_name: impl Into<String>) {
        self.model_name = model_name.into();
    }

    /// The agent's currently-configured temperature.
    pub fn temperature(&self) -> f64 {
        self.temperature
    }

    /// The agent's loaded workflows, if any.
    pub fn workflows(&self) -> &[tinyskills::Skill] {
        &self.workflows
    }

    /// Active Composio integrations fetched at session start.
    pub fn connected_integrations(&self) -> &[crate::integrations::ConnectedIntegration] {
        &self.connected_integrations
    }

    /// This session's transcript key — `{unix_ts}_{token}_{agent}`, unique
    /// per session host, generated once at build time. Sub-agents chain
    /// this into their own transcript filenames so the parent → child
    /// hierarchy is visible on disk.
    pub fn session_key(&self) -> &str {
        &self.session_key
    }

    /// The ancestor chain of session keys for a sub-agent, joined with
    /// `__`. `None` for a root session. Root + prefix together produce
    /// the full transcript stem.
    pub fn session_parent_prefix(&self) -> Option<&str> {
        self.session_parent_prefix.as_deref()
    }

    /// Replace the agent's connected integrations (e.g. from a cached
    /// fetch result when the agent was built outside the normal turn loop).
    pub fn set_connected_integrations(
        &mut self,
        integrations: Vec<crate::integrations::ConnectedIntegration>,
    ) {
        self.connected_integrations = integrations;
        self.connected_integrations_initialized = true;
        self.last_seen_integrations_hash =
            crate::integrations::composio::connected_set_hash(&self.connected_integrations);
    }

    /// The agent's runtime config snapshot.
    pub fn agent_config(&self) -> &crate::config::AgentConfig {
        &self.config
    }

    /// Override the agent's tool-iteration cap after construction.
    ///
    /// Issue #4868 — `build_session_agent_inner` now stamps every agent with
    /// its `AgentDefinition::effective_max_iterations()`, which is the correct
    /// behavior for direct-invocation call sites. A handful of callers need a
    /// *different* cap than the definition's declared budget (e.g. long-running
    /// workflow runs that intentionally exceed any single
    /// agent's normal budget). Those callers should apply their override
    /// AFTER construction via this setter, so the shared definition-cap logic
    /// in the builder doesn't get silently clobbered by pre-construction
    /// mutations (and vice versa).
    pub fn set_max_tool_iterations(&mut self, cap: usize) {
        self.config.max_tool_iterations = cap;
    }

    /// Returns a presentation projection of the runtime-owned history.
    ///
    /// This intentionally returns an owned value: retaining a second borrowed
    /// or mutable `TranscriptEntry` accumulator in the host would recreate
    /// the session state now owned by `tinyagents_runtime::Session`.
    pub fn history(&self) -> Vec<TranscriptEntry> {
        self.runtime_session
            .as_ref()
            .map(|session| {
                crate::agent::message_convert::messages_to_history_projection(session.history())
            })
            .unwrap_or_default()
    }
}
