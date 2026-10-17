//! The parent-execution snapshot a turn hands to the sub-agents it spawns.

use super::*;

impl OpenHumanTurnPrelude {
    /// `inherited` is the parent this turn itself runs under
    /// (`run_context.parent`), when the turn is a sub-agent's; its workspace
    /// descriptor takes precedence over the session's own.
    pub(in crate::agent::session_host) fn parent_context(
        &self,
        inherited: Option<&crate::agent::harness::ParentExecutionContext>,
    ) -> crate::agent::harness::ParentExecutionContext {
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (workflows, memory_context, connected_integrations) = {
            let mutable = self
                .mutable
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            (
                mutable.workflows.clone(),
                mutable.last_memory_context.clone(),
                mutable.connected_integrations.clone(),
            )
        };
        crate::agent::harness::ParentExecutionContext {
            agent_definition_id: self.agent_definition_id.clone(),
            allowed_subagent_ids: self.allowed_subagent_ids.clone(),
            turn_model_source: self.turn_model_source.clone(),
            all_tools: surface.tools.clone(),
            all_tool_specs: surface.durable_tool_specs.clone(),
            visible_tool_names: surface.visible_tool_names.clone(),
            visible_tool_specs: surface.visible_tool_specs.clone(),
            subagent_tool_ceiling_names: self.subagent_tool_ceiling_names.clone(),
            model_name: self.model_name.clone(),
            temperature: self.temperature,
            workspace_dir: self.workspace_dir.clone(),
            workspace_descriptor: inherited
                .and_then(|parent| parent.workspace_descriptor.clone())
                .or_else(|| self.workspace_descriptor.clone()),
            agent_config: self.config.clone(),
            workflows: Arc::new(workflows),
            memory_context: Arc::new(memory_context),
            session_id: self.event_session_id.clone(),
            channel: self.event_channel.clone(),
            connected_integrations,
            tool_call_format: self.tool_dispatcher.tool_call_format(),
            session_key: self.session_key.clone(),
            session_parent_prefix: self.session_parent_prefix.clone(),
            on_progress: self.on_progress.clone(),
            run_queue: self.run_queue.clone(),
            host_env: self.host_env.clone(),
        }
    }
}
