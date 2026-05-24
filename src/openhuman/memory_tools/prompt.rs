//! Prompt section that injects tool-scoped memory rules into the
//! system prompt.
//!
//! ## Why a prompt section
//!
//! Mid-session compression rewrites the rolling chat buffer but never
//! the system prompt — that prompt is frozen for the whole session by
//! design (so the inference backend's prefix cache stays warm; see
//! [`crate::openhuman::agent::prompts::SystemPromptBuilder::build`]).
//!
//! Anything we want to be **compression-resistant** therefore has to
//! live in the system prompt. That is exactly where Critical and High
//! priority [`ToolMemoryRule`]s belong: a "never email Sarah" rule
//! cannot be silently dropped when the buffer fills up.
//!
//! ## What gets rendered
//!
//! The section takes ownership of the caller-supplied list of rules
//! (already filtered to the eager priorities by
//! [`ToolMemoryStore::rules_for_prompt`]) at construction time, mirrors
//! the pattern used by
//! [`crate::openhuman::agent::prompts::ReflectionMemoryContextSection`].
//! Snapshot semantics — the rendered bytes are stable for the lifetime
//! of the session, preserving the inference backend's prefix cache hit.
//!
//! [`ToolMemoryRule`]: super::types::ToolMemoryRule
//! [`ToolMemoryStore::rules_for_prompt`]: super::store::ToolMemoryStore::rules_for_prompt

use anyhow::Result;

use super::types::{ToolMemoryPriority, ToolMemoryRule};
use crate::openhuman::context::prompt::{PromptContext, PromptSection};

/// Heading injected when at least one rule is present.
pub const TOOL_MEMORY_HEADING: &str = "## Tool-scoped rules";

/// Prompt section that renders an at-construction snapshot of
/// [`ToolMemoryRule`]s into the system prompt.
///
/// Construct via [`Self::new`] with the rules the session builder
/// pre-fetched from [`ToolMemoryStore::rules_for_prompt`].
///
/// [`ToolMemoryStore::rules_for_prompt`]: super::store::ToolMemoryStore::rules_for_prompt
pub struct ToolMemoryRulesSection {
    rendered: String,
}

impl ToolMemoryRulesSection {
    /// Build a section from a pre-fetched rule snapshot.
    ///
    /// Rendering happens up-front so subsequent `build` calls — which
    /// run once per system prompt assembly — are I/O-free and
    /// deterministic.
    pub fn new(rules: Vec<ToolMemoryRule>) -> Self {
        Self {
            rendered: render_tool_memory_rules(&rules),
        }
    }

    /// Construct an empty section. Useful as a placeholder for builders
    /// that always include the section name in their chain.
    pub fn empty() -> Self {
        Self {
            rendered: String::new(),
        }
    }

    /// Returns true when the section will emit no output.
    pub fn is_empty(&self) -> bool {
        self.rendered.trim().is_empty()
    }
}

impl PromptSection for ToolMemoryRulesSection {
    fn name(&self) -> &str {
        "tool_memory_rules"
    }

    fn build(&self, _ctx: &PromptContext<'_>) -> Result<String> {
        Ok(self.rendered.clone())
    }
}

/// Pure rendering helper — public so callers that pre-render the block
/// (e.g. tests, dynamic prompt sources) can share the same logic.
pub fn render_tool_memory_rules(rules: &[ToolMemoryRule]) -> String {
    if rules.is_empty() {
        return String::new();
    }

    // Stable order: Critical first, then High; within a priority, by
    // tool name, then by rule body. Callers may pass an already-sorted
    // list (the store does), but rendering must not depend on that
    // contract — the system prompt has to be byte-stable.
    let mut sorted: Vec<&ToolMemoryRule> = rules.iter().collect();
    sorted.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then_with(|| a.tool_name.cmp(&b.tool_name))
            .then_with(|| a.rule.cmp(&b.rule))
            .then_with(|| a.id.cmp(&b.id))
    });

    let mut out = String::new();
    out.push_str(TOOL_MEMORY_HEADING);
    out.push_str("\n\n");
    out.push_str(
        "These rules are pinned by the user or by the safety pipeline. Treat \
        every entry as a hard constraint when considering the matching tool — \
        do not override them silently. Lower-priority guidance lives in the \
        `tool-{name}` memory namespace and can be queried via `memory_recall` \
        if needed.\n\n",
    );

    let mut current_tool: Option<&str> = None;
    for rule in sorted {
        if current_tool != Some(rule.tool_name.as_str()) {
            if current_tool.is_some() {
                out.push('\n');
            }
            out.push_str("### `");
            out.push_str(rule.tool_name.as_str());
            out.push_str("`\n");
            current_tool = Some(rule.tool_name.as_str());
        }
        out.push_str("- ");
        out.push_str(priority_marker(rule.priority));
        out.push(' ');
        out.push_str(rule.rule.trim());
        out.push('\n');
    }

    out
}

fn priority_marker(priority: ToolMemoryPriority) -> &'static str {
    match priority {
        ToolMemoryPriority::Critical => "**[critical]**",
        ToolMemoryPriority::High => "**[high]**",
        ToolMemoryPriority::Normal => "**[normal]**",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::prompts::types::{
        LearnedContextData, PromptContext, ToolCallFormat,
    };
    use crate::openhuman::memory::tool_memory::types::ToolMemorySource;

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
    fn renders_empty_when_no_rules() {
        assert!(render_tool_memory_rules(&[]).is_empty());
    }

    #[test]
    fn section_empty_returns_blank_build_output() {
        let section = ToolMemoryRulesSection::empty();
        assert!(section.is_empty());
    }

    #[test]
    fn renders_heading_and_priority_markers() {
        let rules = vec![
            rule("email", "never email Sarah", ToolMemoryPriority::Critical),
            rule("shell", "avoid sudo", ToolMemoryPriority::High),
        ];
        let out = render_tool_memory_rules(&rules);
        assert!(out.contains(TOOL_MEMORY_HEADING));
        assert!(out.contains("### `email`"));
        assert!(out.contains("### `shell`"));
        assert!(out.contains("**[critical]**"));
        assert!(out.contains("**[high]**"));
        assert!(out.contains("never email Sarah"));
        assert!(out.contains("avoid sudo"));
    }

    #[test]
    fn renders_critical_before_high_regardless_of_input_order() {
        let rules = vec![
            rule("shell", "avoid sudo", ToolMemoryPriority::High),
            rule("email", "never email Sarah", ToolMemoryPriority::Critical),
        ];
        let out = render_tool_memory_rules(&rules);
        let critical_pos = out.find("never email Sarah").unwrap();
        let high_pos = out.find("avoid sudo").unwrap();
        assert!(
            critical_pos < high_pos,
            "Critical rules must render before High; output:\n{out}"
        );
    }

    #[test]
    fn renders_byte_stable_output_for_identical_inputs() {
        let rules = vec![
            rule("email", "never email Sarah", ToolMemoryPriority::Critical),
            rule("shell", "avoid sudo", ToolMemoryPriority::High),
        ];
        let first = render_tool_memory_rules(&rules);
        let again = render_tool_memory_rules(&rules);
        assert_eq!(first, again);
    }

    #[test]
    fn section_renders_via_prompt_section_trait() {
        // build() must not depend on PromptContext fields — it returns
        // the at-construction snapshot verbatim. We call it here to
        // exercise the trait contract directly.
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
        };
        let built = section.build(&ctx).unwrap();
        assert!(built.contains("never email Sarah"));
    }
}
