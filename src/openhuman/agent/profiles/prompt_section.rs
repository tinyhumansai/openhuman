//! Prompt section that injects a profile's free-form persona blurb, plus the
//! optional cross-profile workspace notice (1b) that tells the model where its
//! dedicated workspace is and that other profiles' directories are off-limits.

use std::path::Path;

use crate::openhuman::agent::context::prompt::{PromptContext, PromptSection};
use anyhow::Result;

/// One-sentence system-prompt notice, mirroring hermes's cross-profile
/// disclosure: names the profile's dedicated workspace and states that other
/// profiles' directories are off-limits. Rendered only when a dedicated
/// workspace is active (the enforcement backstop is the guard in
/// [`crate::openhuman::agent::profiles::guard`]).
pub fn cross_profile_workspace_notice(profile_id: &str, workspace_path: &Path) -> String {
    format!(
        "Your dedicated workspace for profile `{profile_id}` is `{}`. Work only there; the \
         directories of other profiles are off-limits.",
        workspace_path.display()
    )
}

/// Renders a profile's `system_prompt_suffix` (or any free-form persona body)
/// as a `## Agent profile` block in the system prompt, optionally followed by
/// the cross-profile workspace notice.
pub struct AgentProfilePromptSection {
    body: String,
    workspace_notice: Option<String>,
}

impl AgentProfilePromptSection {
    pub fn new(body: String) -> Self {
        Self {
            body,
            workspace_notice: None,
        }
    }

    /// Attach the cross-profile workspace notice (1b). Rendered under the
    /// persona body — or on its own when the body is empty — so a
    /// dedicated-workspace profile always discloses its boundary even without a
    /// custom persona suffix.
    #[must_use]
    pub fn with_workspace_notice(mut self, notice: String) -> Self {
        let notice = notice.trim().to_string();
        self.workspace_notice = (!notice.is_empty()).then_some(notice);
        self
    }
}

impl PromptSection for AgentProfilePromptSection {
    fn name(&self) -> &str {
        "agent_profile"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        let body = self.body.trim();
        let notice = self.workspace_notice.as_deref().unwrap_or_default();
        let mut parts: Vec<&str> = Vec::new();
        if !body.is_empty() {
            parts.push(body);
        }
        if !notice.is_empty() {
            parts.push(notice);
        }
        if parts.is_empty() {
            return Ok(String::new());
        }
        Ok(format!("## Agent profile\n\n{}", parts.join("\n\n")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::context::prompt::{
        LearnedContextData, PromptContext, ToolCallFormat,
    };
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
            agents_md_global: None,
            agents_md_local: None,
        };
        let rendered = section.build(&ctx).expect("render profile section");
        assert!(rendered.starts_with("## Agent profile"));
        assert!(rendered.contains("Be concise."));

        let empty = AgentProfilePromptSection::new("   ".into());
        assert_eq!(empty.build(&ctx).expect("empty profile section"), "");
    }

    fn empty_ctx<'a>(visible: &'a HashSet<String>) -> PromptContext<'a> {
        PromptContext {
            workspace_dir: std::path::Path::new("/tmp"),
            model_name: "test-model",
            agent_id: "orchestrator",
            tools: &[],
            workflows: &[],
            dispatcher_instructions: "",
            learned: LearnedContextData::default(),
            visible_tool_names: visible,
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
            agents_md_global: None,
            agents_md_local: None,
        }
    }

    #[test]
    fn cross_profile_workspace_notice_names_path_and_boundary() {
        let notice =
            cross_profile_workspace_notice("alice", std::path::Path::new("/act/profiles/alice"));
        assert!(notice.contains("alice"));
        assert!(notice.contains("/act/profiles/alice"));
        assert!(notice.contains("off-limits"));
    }

    #[test]
    fn workspace_notice_renders_with_and_without_body() {
        let visible = HashSet::new();
        let ctx = empty_ctx(&visible);

        // Notice-only (no persona body) still emits the block + the notice.
        let notice_only = AgentProfilePromptSection::new(String::new())
            .with_workspace_notice("boundary sentence.".into());
        let rendered = notice_only.build(&ctx).expect("render notice-only");
        assert!(rendered.starts_with("## Agent profile"));
        assert!(rendered.contains("boundary sentence."));

        // Body + notice: both present, body first.
        let both = AgentProfilePromptSection::new("Be terse.".into())
            .with_workspace_notice("boundary sentence.".into());
        let rendered = both.build(&ctx).expect("render both");
        let body_at = rendered.find("Be terse.").expect("body present");
        let notice_at = rendered.find("boundary sentence.").expect("notice present");
        assert!(body_at < notice_at, "persona body must precede the notice");

        // A blank notice is dropped, leaving only the body.
        let blank_notice =
            AgentProfilePromptSection::new("Be terse.".into()).with_workspace_notice("  ".into());
        let rendered = blank_notice.build(&ctx).expect("render blank notice");
        assert!(rendered.contains("Be terse."));
        assert!(!rendered.contains("off-limits"));
    }
}
