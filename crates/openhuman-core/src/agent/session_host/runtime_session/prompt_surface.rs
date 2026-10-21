//! Shared construction and refresh for the host turn prelude.

use super::*;

impl OpenHumanSessionHost {
    pub(super) fn create_turn_prelude(
        &self,
        force_integration_refresh: bool,
    ) -> OpenHumanTurnPrelude {
        OpenHumanTurnPrelude {
            config: self.config.clone(),
            prompt_builder: self.prompt_builder.clone(),
            context_config: self.context_config.clone(),
            tool_policy: self.tool_policy.clone(),
            pack_registry_handle: self.pack_registry_handle.clone(),
            tool_dispatcher: self.tool_dispatcher.clone(),
            workspace_dir: self.workspace_dir.clone(),
            action_dir: self.action_dir.clone(),
            model_name: self.model_name.clone(),
            agent_definition_name: self.agent_definition_name.clone(),
            omit_memory_context: self.omit_memory_context,
            thread_id: self.thread_id.clone(),
            agent_definition_id: self.agent_definition_id.clone(),
            event_session_id: self.event_session_id.clone(),
            event_channel: self.event_channel.clone(),
            subagent_tool_ceiling_names: self.subagent_tool_ceiling_names.clone(),
            turn_model_source: self.turn_model_source.clone(),
            temperature: self.temperature,
            workspace_descriptor: self.workspace_descriptor.clone(),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
            on_progress: self.on_progress.clone(),
            run_queue: self.run_queue.clone(),
            host_env: self.host_env.clone(),
            allowed_subagent_ids: self
                .resolved_definition()
                .map(|definition| definition.allowed_subagent_ids().into_iter().collect())
                .unwrap_or_default(),
            sandbox_mode: self
                .resolved_definition()
                .map(|definition| definition.sandbox_mode)
                .unwrap_or(tinyagents_definition::SandboxMode::None),
            definition: self.resolved_definition(),
            runtime_config: self.runtime_config.clone(),
            tool_rules: self.session_tool_rules(),
            tool_surface: Arc::new(std::sync::Mutex::new(OpenHumanTurnToolSurface {
                tools: self.tools.clone(),
                synthesized_tools: self.synthesized_tools.clone(),
                tool_specs: self.tool_specs.clone(),
                durable_tool_specs: self.durable_tool_specs.clone(),
                visible_tool_specs: self.visible_tool_specs.clone(),
                visible_tool_names: self.visible_tool_names.clone(),
                deferred_tool_names: self.deferred_tool_names.clone(),
                permanent_tool_names: self.permanent_tool_names.clone(),
                discovery_enabled: self.discovery_enabled,
                requested_deferred_tools: self.requested_deferred_tools.clone(),
                auto_include_new_synthesized_tools: true,
                synthesized_tool_names: self.synthesized_tool_names.clone(),
                tool_policy_session: self.tool_policy_session.clone(),
                event_session_id: self.event_session_id.clone(),
                event_channel: self.event_channel.clone(),
                agent_definition_name: self.agent_definition_name.clone(),
            })),
            mutable: Arc::new(std::sync::Mutex::new(OpenHumanTurnPreludeMutable {
                last_memory_context: self.last_memory_context.clone(),
                announced_integrations: self.announced_integrations.clone(),
                pending_integration_announcement: self.pending_integration_announcement.clone(),
                announced_mcp_servers: self.announced_mcp_servers.clone(),
                pending_mcp_announcement: self.pending_mcp_announcement.clone(),
                #[cfg(feature = "mcp")]
                mcp_tools: prelude_integrations::McpToolCatalog::default(),
                announced_skills: self.announced_skills.clone(),
                pending_skill_announcement: self.pending_skill_announcement.clone(),
                pending_skill_retraction: self.pending_skill_retraction.clone(),
                connected_integrations: self.connected_integrations.clone(),
                connected_integrations_initialized: self.connected_integrations_initialized
                    && !force_integration_refresh,
                // Builder-provided integrations are not yet verified this session.
                connected_integrations_authoritative: false,
                integration_announcements_seeded: false,
                recorded_integration_actions: Vec::new(),
                recorded_repl_tools: Vec::new(),
                recorded_tool_snapshot_adopted: false,
                workflows: self.workflows.clone(),
                composio_events: None,
                skill_events: None,
                pending_user_text: None,
                pending_memory_turn: None,
            })),
        }
    }

    /// Refresh the same integration and delegation surface used by a live turn.
    /// Prompt dumps and MCP tool listings use this without starting a model turn.
    pub async fn refresh_prompt_surface(&mut self) {
        let prelude = self.create_turn_prelude(true);
        if let Err(error) = prelude.refresh_turn_boundary(true).await {
            log::warn!("[session] prompt surface refresh failed: {error}");
        }
        let mutable = prelude
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.connected_integrations = mutable.connected_integrations.clone();
        self.connected_integrations_initialized = mutable.connected_integrations_initialized;
        drop(mutable);

        let surface = prelude
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.synthesized_tools = surface.synthesized_tools.clone();
        self.tool_specs = surface.tool_specs.clone();
        self.visible_tool_specs = surface.visible_tool_specs.clone();
        self.visible_tool_names = surface.visible_tool_names.clone();
        self.deferred_tool_names = surface.deferred_tool_names.clone();
        self.synthesized_tool_names = surface.synthesized_tool_names.clone();
        self.tool_policy_session = surface.tool_policy_session.clone();
        self.sync_runtime_tool_surface();
    }
}
