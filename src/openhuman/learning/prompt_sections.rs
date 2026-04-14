//! Prompt sections that inject learned context into the agent's system prompt.
//!
//! These sections read pre-fetched data from `PromptContext.learned` — no async
//! or blocking I/O happens during prompt building.

use crate::openhuman::context::prompt::{PromptContext, PromptSection};
use anyhow::Result;

/// Injects recent observations and patterns from the learning subsystem.
pub struct LearnedContextSection;

impl LearnedContextSection {
    pub fn new(_memory: std::sync::Arc<dyn crate::openhuman::memory::Memory>) -> Self {
        // Memory parameter kept for API compatibility but data comes from PromptContext.learned
        Self
    }
}

impl PromptSection for LearnedContextSection {
    fn name(&self) -> &str {
        "learned_context"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.observations.is_empty() && ctx.learned.patterns.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## Learned Context\n\n");

        if !ctx.learned.observations.is_empty() {
            out.push_str("### Recent Observations\n");
            for obs in &ctx.learned.observations {
                out.push_str("- ");
                out.push_str(obs);
                out.push('\n');
            }
            out.push('\n');
        }

        if !ctx.learned.patterns.is_empty() {
            out.push_str("### Recognized Patterns\n");
            for pat in &ctx.learned.patterns {
                out.push_str("- ");
                out.push_str(pat);
                out.push('\n');
            }
            out.push('\n');
        }

        Ok(out)
    }
}

/// Injects the learned user profile into the system prompt.
pub struct UserProfileSection;

impl UserProfileSection {
    pub fn new(_memory: std::sync::Arc<dyn crate::openhuman::memory::Memory>) -> Self {
        Self
    }
}

impl PromptSection for UserProfileSection {
    fn name(&self) -> &str {
        "user_profile"
    }

    fn build(&self, ctx: &PromptContext<'_>) -> Result<String> {
        if ctx.learned.user_profile.is_empty() {
            return Ok(String::new());
        }

        let mut out = String::from("## User Profile (Learned)\n\n");
        for entry in &ctx.learned.user_profile {
            out.push_str("- ");
            out.push_str(entry);
            out.push('\n');
        }
        out.push('\n');
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::context::prompt::LearnedContextData;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
    use async_trait::async_trait;
    use std::collections::HashSet;
    use std::path::Path;
    use std::sync::Arc;

    struct NoopMemory;

    #[async_trait]
    impl Memory for NoopMemory {
        fn name(&self) -> &str {
            "noop"
        }

        async fn store(
            &self,
            _key: &str,
            _content: &str,
            _category: MemoryCategory,
            _session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(None)
        }

        async fn list(
            &self,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn forget(&self, _key: &str) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(0)
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    fn prompt_context(learned: LearnedContextData) -> PromptContext<'static> {
        let visible_tool_names = Box::leak(Box::new(HashSet::new()));
        PromptContext {
            workspace_dir: Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "",
            tools: &[],
            skills: &[],
            dispatcher_instructions: "",
            learned,
            visible_tool_names,
            tool_call_format: crate::openhuman::context::prompt::ToolCallFormat::PFormat,
            connected_integrations: &[],
            include_profile: false,
            include_memory_md: false,
        }
    }

    #[test]
    fn learned_context_section_renders_observations_and_patterns() {
        let section = LearnedContextSection::new(Arc::new(NoopMemory));
        let rendered = section
            .build(&prompt_context(LearnedContextData {
                observations: vec!["Tool use succeeded".into()],
                patterns: vec!["User prefers terse replies".into()],
                user_profile: Vec::new(),
                tree_root_summaries: Vec::new(),
            }))
            .unwrap();

        assert_eq!(section.name(), "learned_context");
        assert!(rendered.contains("## Learned Context"));
        assert!(rendered.contains("### Recent Observations"));
        assert!(rendered.contains("- Tool use succeeded"));
        assert!(rendered.contains("### Recognized Patterns"));
        assert!(rendered.contains("- User prefers terse replies"));
    }

    #[test]
    fn learned_context_section_returns_empty_without_entries() {
        let section = LearnedContextSection::new(Arc::new(NoopMemory));
        assert!(section
            .build(&prompt_context(LearnedContextData::default()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn user_profile_section_renders_bullets() {
        let section = UserProfileSection::new(Arc::new(NoopMemory));
        let rendered = section
            .build(&prompt_context(LearnedContextData {
                observations: Vec::new(),
                patterns: Vec::new(),
                user_profile: vec![
                    "Timezone: America/Los_Angeles".into(),
                    "Prefers Rust".into(),
                ],
                tree_root_summaries: Vec::new(),
            }))
            .unwrap();

        assert_eq!(section.name(), "user_profile");
        assert!(rendered.starts_with("## User Profile (Learned)\n\n"));
        assert!(rendered.contains("- Timezone: America/Los_Angeles"));
        assert!(rendered.contains("- Prefers Rust"));
    }

    #[test]
    fn user_profile_section_returns_empty_without_profile_entries() {
        let section = UserProfileSection::new(Arc::new(NoopMemory));
        assert!(section
            .build(&prompt_context(LearnedContextData::default()))
            .unwrap()
            .is_empty());
    }
}
