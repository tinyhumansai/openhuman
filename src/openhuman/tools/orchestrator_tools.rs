//! Dynamic orchestrator tool generation.
//!
//! The orchestrator agent doesn't directly execute work — it routes it to
//! specialised sub-agents. Rather than exposing a single generic
//! `spawn_subagent(agent_id, prompt)` mega-tool, we synthesise one named
//! tool per entry in the orchestrator's `subagents = [...]` TOML field,
//! so the LLM's function-calling schema contains discoverable, well-named
//! tools like `research`, `plan`, `run_code`, `delegate_gmail`,
//! `delegate_github`, etc.
//!
//! Each synthesised tool's description is pulled live from the target
//! agent's [`AgentDefinition::when_to_use`] (for
//! [`SubagentEntry::AgentId`]) or from the connected Composio toolkit
//! metadata (for [`SubagentEntry::Skills`] wildcard expansions) — so
//! descriptions automatically stay in sync with the definitions and
//! never drift from a hardcoded table.
//!
//! Called from [`crate::openhuman::agent::harness::session::builder`] at
//! agent-build time, with the orchestrator's own definition, the global
//! registry (for delegation target lookups), and the current list of
//! connected Composio integrations.
//!
//! [`AgentDefinition::when_to_use`]: crate::openhuman::agent::harness::definition::AgentDefinition::when_to_use
//! [`SubagentEntry::AgentId`]: crate::openhuman::agent::harness::definition::SubagentEntry::AgentId
//! [`SubagentEntry::Skills`]: crate::openhuman::agent::harness::definition::SubagentEntry::Skills

use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, SubagentEntry,
};
use crate::openhuman::context::prompt::ConnectedIntegration;

use super::{ArchetypeDelegationTool, SkillDelegationTool, Tool};

/// Synthesise the delegation tool list for an agent based on its
/// declarative `subagents` field.
///
/// Each [`SubagentEntry::AgentId`] is resolved against `registry` and
/// rendered as an [`ArchetypeDelegationTool`] whose `name()` defaults to
/// `delegate_{target.id}` (overridable via the target agent's
/// `delegate_name` field) and whose `description()` is the target's
/// `when_to_use` — so editing an agent's TOML description immediately
/// updates the tool schema the orchestrator LLM sees, with zero drift.
///
/// Each [`SubagentEntry::Skills`] wildcard expands to one
/// [`SkillDelegationTool`] per connected Composio integration in
/// `connected_integrations`. The synthesised tool routes to the generic
/// `skills_agent` with `skill_filter = Some("{toolkit_slug}")` pre-set.
///
/// Entries that reference unknown agent ids (not in the registry) are
/// logged at `warn` and skipped — the orchestrator still builds, just
/// without the broken delegation. Entries that reference Skills wildcards
/// with an empty `connected_integrations` slice produce zero tools, which
/// is the correct behaviour when the user has not yet connected any
/// integrations (the LLM should not see phantom `delegate_gmail` tools
/// for unconnected toolkits).
///
/// Returns an empty Vec when `definition.subagents` is empty — callers
/// (notably the builder) handle this by not extending the visible-tool
/// set, so non-delegating agents behave identically to how they did
/// before this module existed.
pub fn collect_orchestrator_tools(
    definition: &AgentDefinition,
    registry: &AgentDefinitionRegistry,
    connected_integrations: &[ConnectedIntegration],
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();

    for entry in &definition.subagents {
        match entry {
            SubagentEntry::AgentId(agent_id) => {
                // Runtime-only sub-agents — the LLM must never see a
                // `delegate_*` tool for these because they're dispatched
                // directly by the runtime, not by an explicit LLM tool
                // call. Issue #574 introduced `summarizer` as the first
                // such sub-agent; future runtime-only agents should
                // join this filter.
                if agent_id == "summarizer" {
                    log::debug!(
                        "[orchestrator_tools] skipping runtime-only sub-agent '{}' \
                         (no delegation tool synthesised)",
                        agent_id
                    );
                    continue;
                }
                let Some(target) = registry.get(agent_id) else {
                    log::warn!(
                        "[orchestrator_tools] subagent '{}' referenced by '{}' is not in the registry — skipping",
                        agent_id,
                        definition.id
                    );
                    continue;
                };
                let tool_name = target
                    .delegate_name
                    .clone()
                    .unwrap_or_else(|| format!("delegate_{}", target.id));
                log::debug!(
                    "[orchestrator_tools] registering archetype delegation tool: {} -> {}",
                    tool_name,
                    target.id
                );
                tools.push(Box::new(ArchetypeDelegationTool {
                    tool_name,
                    agent_id: target.id.clone(),
                    tool_description: target.when_to_use.clone(),
                }));
            }
            SubagentEntry::Skills(wildcard) => {
                if !wildcard.matches_all() {
                    log::warn!(
                        "[orchestrator_tools] subagent skills wildcard '{}' referenced by '{}' is not supported (only \"*\") — skipping",
                        wildcard.skills,
                        definition.id
                    );
                    continue;
                }
                for integration in connected_integrations {
                    // Slug the toolkit name into a tool-name-safe form.
                    // Composio toolkit slugs are already lowercase / dash-
                    // separated (e.g. "gmail", "google_calendar"), but
                    // we guard against surprises so a quirky slug can
                    // never produce an invalid function-calling schema.
                    let slug = sanitise_slug(&integration.toolkit);
                    let tool_name = format!("delegate_{}", slug);
                    // Prefer the toolkit's own one-line description when
                    // available; fall back to a generic template so the
                    // LLM still gets a meaningful tool description even
                    // on brand-new or poorly-populated toolkits.
                    let description = if integration.description.trim().is_empty() {
                        format!(
                            "Delegate to the skills agent with the `{}` integration pre-selected.",
                            integration.toolkit
                        )
                    } else {
                        format!(
                            "Delegate to the skills agent using `{}`. {}",
                            integration.toolkit, integration.description
                        )
                    };
                    log::debug!(
                        "[orchestrator_tools] registering skill delegation tool: {} -> skills_agent (skill_filter={})",
                        tool_name,
                        slug
                    );
                    tools.push(Box::new(SkillDelegationTool {
                        tool_name,
                        skill_id: slug,
                        tool_description: description,
                    }));
                }
            }
        }
    }

    log::info!(
        "[orchestrator_tools] assembled {} delegation tool(s) for agent '{}' ({} integrations connected)",
        tools.len(),
        definition.id,
        connected_integrations.len()
    );

    tools
}

/// Produce a tool-name-safe slug from a free-form integration id.
/// Allows ASCII alphanumerics and underscores; everything else becomes
/// an underscore. OpenAI-style function names only accept
/// `[a-zA-Z0-9_-]{1,64}`, so this is the conservative subset.
fn sanitise_slug(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::agent::harness::definition::{
        DefinitionSource, ModelSpec, PromptSource, SandboxMode, SkillsWildcard, ToolScope,
    };

    fn def(id: &str, when_to_use: &str, delegate_name: Option<&str>) -> AgentDefinition {
        AgentDefinition {
            id: id.into(),
            when_to_use: when_to_use.into(),
            display_name: None,
            system_prompt: PromptSource::Inline(String::new()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: true,
            omit_memory_md: true,
            model: ModelSpec::Inherit,
            temperature: 0.4,
            tools: ToolScope::Wildcard,
            disallowed_tools: vec![],
            skill_filter: None,
            category_filter: None,
            max_iterations: 8,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            uses_fork_context: false,
            subagents: vec![],
            delegate_name: delegate_name.map(String::from),
            source: DefinitionSource::Builtin,
        }
    }

    /// A real orchestrator definition that delegates to two named agents
    /// (one with an explicit `delegate_name`, one without) plus a skills
    /// wildcard. Exercises every branch of `collect_orchestrator_tools`.
    fn sample_orchestrator() -> AgentDefinition {
        let mut orch = def("orchestrator", "Routes work to the right specialist", None);
        orch.subagents = vec![
            SubagentEntry::AgentId("researcher".into()),
            SubagentEntry::AgentId("archivist".into()),
            SubagentEntry::Skills(SkillsWildcard { skills: "*".into() }),
        ];
        orch
    }

    fn registry_with_targets() -> AgentDefinitionRegistry {
        let mut reg = AgentDefinitionRegistry::default();
        reg.insert(def(
            "researcher",
            "Web & docs crawler — reads real documentation",
            Some("research"),
        ));
        // `archivist` has no `delegate_name` override — tool name should
        // fall back to `delegate_archivist`.
        reg.insert(def(
            "archivist",
            "Background librarian — extracts lessons from a completed session",
            None,
        ));
        reg
    }

    fn integration(toolkit: &str, description: &str) -> ConnectedIntegration {
        ConnectedIntegration {
            toolkit: toolkit.into(),
            description: description.into(),
            tools: vec![],
            connected: true,
        }
    }

    /// Baseline: an orchestrator with 2 AgentId entries + a Skills
    /// wildcard, against a registry that knows both targets and a
    /// connected_integrations list with three toolkits, should produce
    /// 2 + 3 = 5 delegation tools, each with the expected name and
    /// description source.
    #[test]
    fn collects_agentid_entries_and_expands_skills_wildcard() {
        let orch = sample_orchestrator();
        let reg = registry_with_targets();
        let integrations = vec![
            integration("gmail", "Send and read email via Gmail."),
            integration("github", "Manage repos, issues, and pull requests."),
            integration("notion", "Read and write pages and databases."),
        ];

        let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();

        assert_eq!(
            names,
            vec![
                "research",              // researcher's delegate_name override
                "delegate_archivist",    // archivist has no delegate_name → default
                "delegate_gmail",
                "delegate_github",
                "delegate_notion",
            ],
            "tool names should come from delegate_name overrides, id fallbacks, and sanitised toolkit slugs"
        );

        // Descriptions should come from when_to_use for archetype tools,
        // and from a templated string mentioning the toolkit display name
        // for skill tools.
        let research_tool = tools.iter().find(|t| t.name() == "research").unwrap();
        assert!(research_tool.description().contains("crawler"));

        let gmail_tool = tools.iter().find(|t| t.name() == "delegate_gmail").unwrap();
        assert!(gmail_tool.description().contains("gmail"));
        assert!(gmail_tool.description().contains("email"));
    }

    /// An orchestrator with a Skills wildcard but no connected
    /// integrations should produce zero skill delegation tools — the LLM
    /// must not be shown phantom `delegate_*` tools for toolkits that
    /// aren't authorised.
    #[test]
    fn skills_wildcard_with_no_integrations_produces_no_tools() {
        let orch = sample_orchestrator();
        let reg = registry_with_targets();
        let tools = collect_orchestrator_tools(&orch, &reg, &[]);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, vec!["research", "delegate_archivist"]);
    }

    /// An AgentId entry that points at an id not present in the registry
    /// should be logged and silently skipped, rather than panicking or
    /// aborting tool assembly. The orchestrator still builds.
    #[test]
    fn unknown_subagent_id_is_skipped_not_fatal() {
        let mut orch = def("orchestrator", "test", None);
        orch.subagents = vec![
            SubagentEntry::AgentId("researcher".into()),
            SubagentEntry::AgentId("ghost_agent_nope".into()),
        ];
        let reg = registry_with_targets();
        let tools = collect_orchestrator_tools(&orch, &reg, &[]);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert_eq!(names, vec!["research"]);
    }

    /// An empty `subagents` list should produce zero tools — regular
    /// non-delegating agents (welcome, code_executor, etc.) reach this
    /// path without any subagents and must not pick up stray tools.
    #[test]
    fn empty_subagents_produces_no_tools() {
        let orch = def("welcome", "First agent", None);
        let reg = registry_with_targets();
        let tools = collect_orchestrator_tools(&orch, &reg, &[]);
        assert!(tools.is_empty());
    }

    /// Toolkit slugs with dashes, spaces, or mixed case should be
    /// normalised to `[a-z0-9_]` before being used as part of a function
    /// name — the OpenAI tool-calling schema has strict character rules.
    #[test]
    fn sanitise_slug_lowercases_and_replaces_invalid_chars() {
        assert_eq!(sanitise_slug("Gmail"), "gmail");
        assert_eq!(sanitise_slug("google-calendar"), "google_calendar");
        assert_eq!(sanitise_slug("slack.bot"), "slack_bot");
        assert_eq!(sanitise_slug("weird name!"), "weird_name_");
    }
}
