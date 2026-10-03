//! History, context, and system prompt management.

use super::super::types::OpenHumanSessionHost;
use crate::agent::prompts::{
    tool_call_format_from_dialect, LearnedContextData, PromptContext, PromptTool,
};
use crate::tools::agent_policy::render_tool_policy_boundary;

use anyhow::Result;

impl OpenHumanSessionHost {
    /// Builds the system prompt for the current turn, including tool
    /// instructions and learned context.
    pub fn build_system_prompt(&self, learned: LearnedContextData) -> Result<String> {
        // `visible_tool_specs` holds shared `Arc<ToolSpec>` leaves. Materialise
        // the canonical dialect input for this prompt build.
        let visible_specs_owned: Vec<tinytools::ToolSpec> = self
            .visible_tool_specs
            .iter()
            .map(|spec| spec.as_ref().clone())
            .collect();
        let instructions = self
            .tool_dispatcher
            .prompt_instructions(&visible_specs_owned);
        // Adapt the agent's whole callable surface into the shared PromptTool
        // shape that every prompt-building call-site uses. Temporary vec
        // borrows from the two tool `Arc`s and lives for the duration of the
        // prompt build. The synthesised delegates belong here: the catalogue
        // this renders is what tells the model a `delegate_*` tool exists.
        let all_tools = self.all_tool_refs();
        let mut prompt_tools = PromptTool::from_tool_refs(all_tools.iter().copied());
        prompt_tools.retain(|tool| !self.permanent_tool_names.contains(tool.name.as_ref()));
        let mut prompt_visible_tool_names =
            self.tool_policy_session.visible_tool_names_for_prompt();
        crate::agent::prompts::swap_deferred_for_discovery_bridge(
            &mut prompt_tools,
            &mut prompt_visible_tool_names,
            &self.deferred_tool_names,
        );
        // Load AGENTS.md instruction layers once per system-prompt build (never
        // re-read per turn — the caller builds the prompt once at session start
        // and reuses the bytes, preserving the frozen-prefix / KV-cache
        // contract). Global layer from the workspace dir; project layer from the
        // effective action dir. Gated by `agents_md_enabled`.
        let agents_md = if self.config.agents_md_enabled {
            crate::agent::prompts::load_agents_md_layers(&self.workspace_dir, &self.action_dir)
        } else {
            tracing::debug!(
                "[agents_md] disabled by config; skipping AGENTS.md injection for main agent"
            );
            crate::agent::prompts::AgentsMdContent::default()
        };
        let ctx = PromptContext {
            workspace_dir: &self.workspace_dir,
            model_name: &self.model_name,
            agent_id: &self.agent_definition_name,
            tools: &prompt_tools,
            workflows: &self.workflows,
            dispatcher_instructions: &instructions,
            learned,
            visible_tool_names: &prompt_visible_tool_names,
            tool_call_format: tool_call_format_from_dialect(
                self.tool_dispatcher.tool_call_format(),
            ),
            connected_integrations: &self.connected_integrations,
            connected_identities_md: crate::agent::prompts::render_connected_identities(),
            include_profile: !self.omit_profile,
            include_memory_md: !self.omit_memory_md,
            curated_snapshot: None,
            user_identity: crate::security::credentials::identity::peek_credential_user_identity(),
            personality_roster: vec![], // TODO: build_personality_roster(&workspace_dir)
            agents_md_global: agents_md.global,
            agents_md_local: agents_md.local,
        };
        // Route through the global context manager so every
        // prompt-building call-site — main agent, sub-agent runner,
        // channel runtimes — shares one builder configuration.
        let prompt = self
            .context
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .build_system_prompt(&ctx)?;
        // Appended, not prepended (#5704). Every line of this block is
        // session-scoped — agent id, channel, entry point, risk level, the
        // allowed-tool list — so putting it first moves the prompt's first
        // diverging byte to offset 0 and costs the inference backend's
        // automatic prefix cache everything behind it. That is the same
        // concern that keeps DateTimeSection out of `for_subagent` and keeps
        // the connected-server overview sorted. The model reads the whole
        // system message either way.
        //
        // It also keeps the archetype/persona as the prompt's opening line,
        // which the prepend had replaced with a constant heading for every
        // agent.
        let boundary = render_tool_policy_boundary(&self.tool_policy_session, 2048);
        Ok(append_tool_policy_boundary(prompt, boundary))
    }
}

/// Place the tool-policy boundary block relative to the assembled prompt.
///
/// Separated from [`OpenHumanSessionHost`] so the ordering can be tested without standing up a
/// session: everything that decides the placement is in these two arguments.
fn append_tool_policy_boundary(prompt: String, boundary: Option<String>) -> String {
    match boundary {
        Some(boundary) => format!("{prompt}\n\n{boundary}"),
        None => prompt,
    }
}
