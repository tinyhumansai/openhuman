//! System prompt builder for the `skill_creator` built-in agent.

use crate::openhuman::context::prompt::{
    render_safety, render_tools, render_user_files, render_workspace, PromptContext,
};
use anyhow::Result;
use tracing::{debug, trace};

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    debug!(
        agent_id = %ctx.agent_id,
        model_name = %ctx.model_name,
        "[skill_creator::prompt] build: start"
    );
    let mut out = String::with_capacity(4096);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    let user_files = match render_user_files(ctx) {
        Ok(user_files) => user_files,
        Err(error) => {
            debug!(%error, "[skill_creator::prompt] build: render_user_files failed");
            return Err(error);
        }
    };
    let has_user_files = !user_files.trim().is_empty();
    trace!(
        has_user_files,
        user_files_len = user_files.len(),
        "[skill_creator::prompt] build: user files rendered"
    );
    if has_user_files {
        out.push_str(user_files.trim_end());
        out.push_str("\n\n");
    }

    let tools = match render_tools(ctx) {
        Ok(tools) => tools,
        Err(error) => {
            debug!(%error, "[skill_creator::prompt] build: render_tools failed");
            return Err(error);
        }
    };
    let has_tools = !tools.trim().is_empty();
    trace!(
        has_tools,
        tools_len = tools.len(),
        "[skill_creator::prompt] build: tools rendered"
    );
    if has_tools {
        out.push_str(tools.trim_end());
        out.push_str("\n\n");
    }

    let safety = render_safety();
    trace!(
        safety_len = safety.len(),
        "[skill_creator::prompt] build: safety rendered"
    );
    out.push_str(safety.trim_end());
    out.push_str("\n\n");

    let workspace = match render_workspace(ctx) {
        Ok(workspace) => workspace,
        Err(error) => {
            debug!(%error, "[skill_creator::prompt] build: render_workspace failed");
            return Err(error);
        }
    };
    let has_workspace = !workspace.trim().is_empty();
    trace!(
        has_workspace,
        workspace_len = workspace.len(),
        "[skill_creator::prompt] build: workspace rendered"
    );
    if has_workspace {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    debug!(
        output_len = out.len(),
        "[skill_creator::prompt] build: done"
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, ToolCallFormat};
    use std::collections::HashSet;

    #[test]
    fn build_returns_nonempty_body() {
        let visible: HashSet<String> = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "skill_creator",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &visible,
            tool_call_format: ToolCallFormat::PFormat,
            connected_integrations: &[],
            connected_identities_md: String::new(),
            include_profile: false,
            include_memory_md: false,
            curated_snapshot: None,
            user_identity: None,
            personality_soul_md: None,
            personality_memory_md: None,
            personality_roster: vec![],
            workflows: &[],
        };
        let body = build(&ctx).unwrap();
        assert!(!body.is_empty());
        assert!(body.contains("Do not assume QuickJS exists."));
    }
}
