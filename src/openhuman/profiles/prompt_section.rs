//! Prompt section that injects a profile's free-form persona blurb.

use crate::openhuman::context::prompt::{PromptContext, PromptSection};
use anyhow::Result;

/// Renders a profile's `system_prompt_suffix` (or any free-form persona body)
/// as a `## Agent profile` block in the system prompt.
pub struct AgentProfilePromptSection {
    body: String,
}

impl AgentProfilePromptSection {
    pub fn new(body: String) -> Self {
        Self { body }
    }
}

impl PromptSection for AgentProfilePromptSection {
    fn name(&self) -> &str {
        "agent_profile"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        if self.body.trim().is_empty() {
            return Ok(String::new());
        }
        Ok(format!("## Agent profile\n\n{}", self.body.trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::{LearnedContextData, PromptContext, ToolCallFormat};
    use std::collections::HashSet;

    #[test]
    fn prompt_section_renders_expected_text() {
        let section = AgentProfilePromptSection::new("  Be concise.  ".into());
        assert_eq!(section.name(), "agent_profile");
        let visible_tool_names = HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "orchestrator",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: &visible_tool_names,
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
        };
        let rendered = section.build(&ctx).expect("render profile section");
        assert!(rendered.starts_with("## Agent profile"));
        assert!(rendered.contains("Be concise."));

        let empty = AgentProfilePromptSection::new("   ".into());
        assert_eq!(empty.build(&ctx).expect("empty profile section"), "");
    }
}
