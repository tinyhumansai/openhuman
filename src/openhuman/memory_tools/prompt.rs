//! Prompt section that injects tool-scoped memory rules into the system
//! prompt — thin host shim over `tinycortex::memory::tool_memory::render` (W7).
//!
//! ## Why a prompt section
//!
//! Mid-session compression rewrites the rolling chat buffer but never the
//! system prompt — that prompt is frozen for the whole session by design (so the
//! inference backend's prefix cache stays warm; see
//! [`crate::openhuman::agent::prompts::SystemPromptBuilder::build`]). Anything we
//! want to be **compression-resistant** therefore has to live in the system
//! prompt — exactly where Critical and High priority [`ToolMemoryRule`]s belong.
//!
//! ## What this shim owns
//!
//! The rendering (`render_tool_memory_rules`) and the section type
//! ([`ToolMemoryRulesSection`], a byte-stable at-construction snapshot) are the
//! crate's and are re-exported here. Host-retained: the [`PromptSection`] impl
//! that plugs the crate section into the host system-prompt builder — a host
//! trait we can implement for the crate type under the orphan rule.
//!
//! [`ToolMemoryRule`]: super::types::ToolMemoryRule

use anyhow::Result;

use crate::openhuman::context::prompt::{PromptContext, PromptSection};

pub use tinycortex::memory::tool_memory::render::{
    render_tool_memory_rules, ToolMemoryRulesSection, TOOL_MEMORY_HEADING,
};

impl PromptSection for ToolMemoryRulesSection {
    fn name(&self) -> &str {
        "tool_memory_rules"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        // build() must not depend on PromptContext fields — it returns the
        // at-construction snapshot verbatim so the inference prefix cache stays warm.
        Ok(self.rendered().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::prompts::types::{
        LearnedContextData, PromptContext, ToolCallFormat,
    };
    use crate::openhuman::memory_tools::types::{
        ToolMemoryPriority, ToolMemoryRule, ToolMemorySource,
    };

    fn rule(tool: &str, body: &str, priority: ToolMemoryPriority) -> ToolMemoryRule {
        ToolMemoryRule {
            id: format!("{tool}/{body}"),
            tool_name: tool.into(),
            rule: body.into(),
            priority,
            source: ToolMemorySource::UserExplicit,
            tags: vec![],
            created_at: "2026-05-11T00:00:00Z".into(),
            updated_at: "2026-05-11T00:00:00Z".into(),
        }
    }

    #[test]
    fn section_empty_returns_blank_build_output() {
        let section = ToolMemoryRulesSection::empty();
        assert!(section.is_empty());
    }

    #[test]
    fn section_renders_via_prompt_section_trait() {
        // Exercise the host PromptSection glue over the crate section: build()
        // returns the at-construction snapshot regardless of PromptContext.
        let section = ToolMemoryRulesSection::new(vec![rule(
            "email",
            "never email Sarah",
            ToolMemoryPriority::Critical,
        )]);
        assert!(!section.is_empty());
        let visible = std::collections::HashSet::new();
        let ctx = PromptContext {
            workspace_dir: std::path::Path::new("."),
            model_name: "test",
            agent_id: "test",
            tools: &[],
            workflows: &[],
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
        };
        let built = section.build(&ctx).unwrap();
        assert!(built.contains("never email Sarah"));
    }
}
