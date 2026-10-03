use super::*;

impl OpenHumanTurnPrelude {
    pub(super) fn build_system_prompt_tiered(
        &self,
        learned: crate::agent::prompts::LearnedContextData,
    ) -> Result<crate::agent::prompts::TieredPrompt> {
        use crate::agent::prompts::{tool_call_format_from_dialect, PromptContext, PromptTool};
        let surface = self
            .tool_surface
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let specs = surface
            .visible_tool_specs
            .iter()
            .filter(|spec| {
                self.thread_id.is_some()
                    || !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(
                        &spec.name,
                    )
            })
            .map(|spec| spec.as_ref().clone())
            .collect::<Vec<_>>();
        let prompt_specs = specs
            .iter()
            .filter(|spec| !surface.permanent_tool_names.contains(&spec.name))
            .cloned()
            .collect::<Vec<_>>();
        let instructions = self.tool_dispatcher.prompt_instructions(&prompt_specs);
        let tool_refs = surface
            .tools
            .iter()
            .chain(surface.synthesized_tools.iter())
            .filter(|tool| {
                self.thread_id.is_some()
                    || !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(
                        tool.name(),
                    )
            })
            .map(|tool| tool.as_ref())
            .collect::<Vec<_>>();
        let mut prompt_tools = PromptTool::from_tool_refs(tool_refs.iter().copied());
        prompt_tools.retain(|tool| !surface.permanent_tool_names.contains(tool.name.as_ref()));
        let mut visible_tool_names = surface.tool_policy_session.visible_tool_names_for_prompt();
        visible_tool_names.retain(|name| !surface.permanent_tool_names.contains(name));
        if self.thread_id.is_none() {
            visible_tool_names.retain(|name| {
                !crate::agent::tinyagents::harness_tool_registration::is_thread_goal_tool(name)
            });
        }
        crate::agent::prompts::swap_deferred_for_discovery_bridge(
            &mut prompt_tools,
            &mut visible_tool_names,
            &surface.deferred_tool_names,
        );
        let agents_md = if self.config.agents_md_enabled {
            crate::agent::prompts::load_agents_md_layers(&self.workspace_dir, &self.action_dir)
        } else {
            Default::default()
        };
        let mutable = self
            .mutable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let context = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            agent_id: &self.agent_definition_name,
            tools: &prompt_tools,
            workflows: &mutable.workflows,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &visible_tool_names,
            tool_call_format: tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            connected_integrations: &mutable.connected_integrations,
            connected_identities_md: crate::agent::prompts::render_connected_identities(),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![],
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        self.context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .build_system_prompt_tiered(&context)
    }
}
