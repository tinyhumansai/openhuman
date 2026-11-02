use super::*;

impl OpenHumanSessionHost {
    // ─────────────────────────────────────────────────────────────────
    // Small accessors used by `run_single` + `turn` + sub-agent runner
    // ─────────────────────────────────────────────────────────────────

    pub(in crate::agent::session_host) fn event_session_id(&self) -> &str {
        &self.event_session_id
    }

    pub(in crate::agent::session_host) fn event_channel(&self) -> &str {
        &self.event_channel
    }

    /// The agent definition id this session is running
    /// (`"welcome"`, `"orchestrator"`, `"workflow_builder"`, …).
    ///
    /// Exposed so callers that build sessions via
    /// [`OpenHumanSessionHost::from_config_for_agent`] can stamp the resolved id onto
    /// correlation logs and progress events without reaching for the
    /// source `Config`. See [`SessionHostBuilder::agent_definition_name`]
    /// for the full list of downstream surfaces (transcript filename,
    /// transcript metadata header, and `PromptContext::agent_id`) that
    /// read this field.
    pub fn agent_definition_name(&self) -> &str {
        &self.agent_definition_name
    }

    /// Returns a new `SessionHostBuilder`.
    pub fn builder() -> SessionHostBuilder {
        SessionHostBuilder::new()
    }

    /// Clone the agent's model source. Used by the sub-agent runner /
    /// parent-context builder to share the parent's provider instance with
    /// spawned sub-agents (so they share connection pools, retry budgets, and
    /// rate-limit state) — issue #4249, Phase 3 / Motion A.
    pub fn turn_model_source(&self) -> crate::agent::tinyagents::TurnModelSource {
        self.turn_model_source.clone()
    }

    /// Borrow the agent's durable tool registry as a slice. Used by the
    /// sub-agent runner to filter the parent's tool registry per-archetype.
    ///
    /// This is **not** the agent's whole callable surface — the synthesised
    /// delegation tools live in their own `Arc`. Use [`Self::all_tool_refs`]
    /// when you mean "every tool this agent can run".
    pub fn tools(&self) -> &[Box<dyn Tool>] {
        self.tools.as_slice()
    }

    /// Clone the agent's durable tools `Arc` for sharing with sub-agents.
    ///
    /// Deliberately excludes [`Self::synthesized_tools_arc`]: a sub-agent must
    /// never be handed a `delegate_*` tool, which the harness re-asserts at
    /// registration time (issue #4452).
    pub fn tools_arc(&self) -> Arc<Vec<Box<dyn Tool>>> {
        Arc::clone(&self.tools)
    }

    /// Every tool this agent can execute: the durable registry first, then the
    /// synthesised delegation set — the same order as [`Self::tool_specs`] and
    /// turn dispatch.
    ///
    /// Borrowed rather than materialised as a `Vec<Box<dyn Tool>>` because
    /// `Box<dyn Tool>` is not cloneable — the two sets can be read together but
    /// never merged into one owned slice.
    pub fn all_tool_refs(&self) -> Vec<&dyn Tool> {
        self.tools
            .iter()
            .chain(self.synthesized_tools.iter())
            .map(|t| t.as_ref())
            .collect()
    }

    /// Borrow the agent's tool specs (pre-serialised). Captured at
    /// turn-start so sub-agents can pass byte-identical schemas to the
    /// provider for prefix-cache reuse.
    pub fn tool_specs(&self) -> &[Arc<ToolSpec>] {
        self.tool_specs.as_slice()
    }

    /// Clone the agent's full tool specs `Arc` (durable and synthesised).
    pub fn tool_specs_arc(&self) -> Arc<Vec<Arc<ToolSpec>>> {
        Arc::clone(&self.tool_specs)
    }

    /// Clone the agent's provider-facing spec list: visible, policy-allowed,
    /// de-duplicated, synthesised delegates included.
    pub fn visible_tool_specs_arc(&self) -> Arc<Vec<Arc<ToolSpec>>> {
        Arc::clone(&self.visible_tool_specs)
    }

    /// Clone the specs of the durable registry alone, index for index with
    /// [`Self::tools_arc`] — the pair a sub-agent is handed, so a child never
    /// sees a spec for a synthesised delegate it holds no instance for.
    pub fn durable_tool_specs_arc(&self) -> Arc<Vec<Arc<ToolSpec>>> {
        Arc::clone(&self.durable_tool_specs)
    }

    /// The full host [`Config`](crate::config::Config) this session
    /// was built with, through the factory or an explicitly configured model.
    ///
    /// `None` on the bare-builder path (`SessionHostBuilder` without
    /// `AgentFactory` or `SessionHostBuilder::chat_model_with_config`), used by
    /// tests and callers assembling a session by hand. Adapters needing config treat
    /// `None` as "not available" rather than loading one itself — see
    /// [`Self::host_capabilities_available`].
    pub fn runtime_config(&self) -> Option<Arc<crate::config::Config>> {
        self.runtime_config.clone()
    }

    /// The definition this session runs under: the one it was built from when
    /// the factory had one, else the process registry's entry for
    /// `agent_definition_id`.
    ///
    /// Prefer this over a bare `AgentDefinitionRegistry::current().get(..)` in
    /// turn-path code that needs the agent's *own* settings (`sandbox_mode`,
    /// `subagents`): a session built from an explicit definition must not have
    /// them replaced by a same-id registry entry.
    pub(crate) fn resolved_definition(
        &self,
    ) -> Option<Arc<crate::agent_registry::host_definition::AgentDefinition>> {
        self.definition.clone().or_else(|| {
            crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
                .and_then(|registry| registry.get(&self.agent_definition_id).cloned())
                .map(Arc::new)
        })
    }
}
