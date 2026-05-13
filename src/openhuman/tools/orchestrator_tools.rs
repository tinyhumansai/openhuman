//! Dynamic orchestrator tool generation.
//!
//! The orchestrator agent is direct-first and only delegates specialised
//! work. Rather than exposing a single generic
//! `spawn_subagent(agent_id, prompt)` mega-tool, we synthesise one named
//! tool per [`SubagentEntry::AgentId`] in the orchestrator's
//! `subagents = [...]` TOML field, so the LLM's function-calling schema
//! contains discoverable, well-named tools like `research`, `plan`,
//! `run_code`, etc.
//!
//! For [`SubagentEntry::Skills`] wildcard expansions (#1335) we synthesise
//! a single collapsed `delegate_to_integrations_agent` tool that takes the
//! toolkit slug as an argument — keeping the orchestrator's schema cost
//! constant in the integration dimension instead of scaling with the
//! number of connected toolkits.
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

// SpawnWorkerThreadTool import kept commented while the worker-thread spawn is
// temporarily disabled (see tinyhumansai/openhuman#1624).
#[allow(unused_imports)]
use super::SpawnWorkerThreadTool;
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
/// Each [`SubagentEntry::Skills`] wildcard expands to a single
/// collapsed [`SkillDelegationTool`] named
/// `delegate_to_integrations_agent` whose `toolkit` argument selects
/// among the slugs of every connected Composio integration in
/// `connected_integrations`. The tool routes to the generic
/// `integrations_agent` with the chosen toolkit's slug passed as
/// `skill_filter`. The collapsed form keeps the orchestrator's
/// function-calling schema constant in the integration dimension
/// (#1335).
///
/// Entries that reference unknown agent ids (not in the registry) are
/// logged at `warn` and skipped — the orchestrator still builds, just
/// without the broken delegation. Entries that reference Skills wildcards
/// with an empty `connected_integrations` slice produce zero tools, which
/// is the correct behaviour when the user has not yet connected any
/// integrations (the LLM should not see a `delegate_to_integrations_agent`
/// tool with an empty enum).
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

    // Orchestrator-only tool: spawn_worker_thread.
    // Temporarily disabled — worker threads do not yet have a proper UI
    // showcase (see tinyhumansai/openhuman#1624). Re-enable once the
    // dedicated worker-thread surface lands.
    // if definition.id == "orchestrator" {
    //     tools.push(Box::new(SpawnWorkerThreadTool::new()));
    // }

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
                let direct_first_description = format!(
                    "Use only when direct response/direct tools are insufficient. {}",
                    target.when_to_use
                );
                tools.push(Box::new(ArchetypeDelegationTool {
                    tool_name,
                    agent_id: target.id.clone(),
                    tool_description: direct_first_description,
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
                // Collapsed delegation tool (#1335). Previously this loop
                // emitted one `delegate_<toolkit>` tool per connected
                // integration. Every one of those tools dispatched to the
                // same `integrations_agent` with a different `skill_filter`,
                // so the fan-out cost the orchestrator schema bytes without
                // buying any new routing capability. We now emit at most
                // one `delegate_to_integrations_agent` tool that takes the
                // toolkit slug as an argument; the description enumerates
                // the connected toolkits so the orchestrator still
                // discovers which integrations are routable.
                // `sanitise_slug` is lossy — `Slack.Bot` and `Slack-Bot`
                // both collapse to `slack_bot`. Once the raw id is
                // discarded, one upstream integration would silently
                // shadow the other. Detect the collision here, drop
                // every duplicate after the first, and warn so routing
                // stays unambiguous (the first arrival keeps the slug;
                // later arrivals are unreachable through this enum and
                // safer to omit than silently re-target).
                let mut connected: Vec<(String, String)> = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for integration in connected_integrations {
                    if !integration.connected {
                        log::debug!(
                            "[orchestrator_tools] skipping unconnected integration: {}",
                            integration.toolkit
                        );
                        continue;
                    }
                    // Slug the toolkit name into a tool-name-safe
                    // (and argument-safe) form so the LLM-facing
                    // enum stays predictable across odd toolkit
                    // names (dashes, dots, spaces, mixed case).
                    let slug = sanitise_slug(&integration.toolkit);
                    if !seen.insert(slug.clone()) {
                        log::warn!(
                            "[orchestrator_tools] duplicate sanitised slug '{slug}' from raw \
                             toolkit '{raw}' — dropping to keep collapsed delegation routing \
                             unambiguous",
                            raw = integration.toolkit
                        );
                        continue;
                    }
                    // Empty integration descriptions otherwise render as a
                    // bare ` - slug` line in the collapsed tool description,
                    // which gives the orchestrator LLM no hint about what
                    // the toolkit actually does. Fall back to the
                    // generic per-toolkit phrasing the old fan-out path
                    // used so brand-new or under-populated toolkits stay
                    // informative.
                    let description = if integration.description.trim().is_empty() {
                        format!(
                            "External integration via {} — see the toolkit docs for available actions.",
                            integration.toolkit
                        )
                    } else {
                        integration.description.clone()
                    };
                    connected.push((slug, description));
                }
                match SkillDelegationTool::for_connected(connected) {
                    Some(tool) => {
                        log::debug!(
                            "[orchestrator_tools] registering collapsed integrations delegation tool ({} toolkits)",
                            tool.connected_toolkits.len()
                        );
                        tools.push(Box::new(tool));
                    }
                    None => {
                        log::debug!(
                            "[orchestrator_tools] no connected integrations — collapsed delegation tool omitted"
                        );
                    }
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
///
/// Used both when synthesising `delegate_*` tools and when rendering the
/// delegation guide in prompts — they must agree on slug canonicalisation
/// so the prompt always references a tool name that actually exists.
pub(crate) fn sanitise_slug(raw: &str) -> String {
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
            extra_tools: vec![],
            max_iterations: 8,
            max_result_chars: None,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
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
    /// 2 archetype tools + 1 collapsed integrations delegation tool
    /// (#1335) — independent of how many integrations are connected.
    #[test]
    fn collects_agentid_entries_and_collapses_skills_wildcard() {
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
                // `spawn_worker_thread` is temporarily disabled upstream —
                // see tinyhumansai/openhuman#1624. Re-add the leading entry
                // when the registration in `collect_orchestrator_tools` is
                // restored.
                "research",           // researcher's delegate_name override
                "delegate_archivist", // archivist has no delegate_name → default
                "delegate_to_integrations_agent",
            ],
            "skills wildcard must collapse to a single delegate_to_integrations_agent tool"
        );

        // Archetype tool descriptions come from `when_to_use`.
        let research_tool = tools.iter().find(|t| t.name() == "research").unwrap();
        assert!(research_tool.description().contains("crawler"));

        // The collapsed delegation tool enumerates every connected toolkit
        // in its description so the orchestrator still discovers what's
        // routable.
        let delegate_tool = tools
            .iter()
            .find(|t| t.name() == "delegate_to_integrations_agent")
            .unwrap();
        let desc = delegate_tool.description();
        assert!(desc.contains("gmail"));
        assert!(desc.contains("github"));
        assert!(desc.contains("notion"));
    }

    /// The collapsed delegation tool's count is constant in the
    /// integration dimension (#1335 primary acceptance criterion).
    #[test]
    fn collapsed_delegation_tool_count_is_constant_across_integration_counts() {
        let orch = sample_orchestrator();
        let reg = registry_with_targets();

        for n in [1usize, 3, 7, 20] {
            let integrations: Vec<_> = (0..n)
                .map(|i| integration(&format!("tool{i}"), &format!("Toolkit number {i}.")))
                .collect();
            let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
            let delegation_count = tools
                .iter()
                .filter(|t| t.name() == "delegate_to_integrations_agent")
                .count();
            assert_eq!(
                delegation_count, 1,
                "expected exactly one collapsed delegation tool for {n} integrations"
            );
        }
    }

    /// An orchestrator with a Skills wildcard but no connected
    /// integrations should produce zero integrations delegation tools —
    /// the LLM must not be shown a routing handle for an empty set.
    #[test]
    fn skills_wildcard_with_no_integrations_produces_no_delegation_tool() {
        let orch = sample_orchestrator();
        let reg = registry_with_targets();
        let tools = collect_orchestrator_tools(&orch, &reg, &[]);
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        // `spawn_worker_thread` is temporarily disabled — see #1624.
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
        // `spawn_worker_thread` is temporarily disabled — see #1624.
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

    /// Unconnected integrations must be silently dropped from the
    /// collapsed delegation tool's enum. Otherwise the orchestrator
    /// could supply `toolkit = "<unconnected>"` and trigger a pre-flight
    /// rejection downstream that says "not connected".
    #[test]
    fn unconnected_integrations_are_omitted_from_collapsed_tool() {
        let orch = sample_orchestrator();
        let reg = registry_with_targets();
        let integrations = vec![
            integration("gmail", "Send and read email."),
            ConnectedIntegration {
                toolkit: "github".into(),
                description: "GitHub access.".into(),
                tools: vec![],
                connected: false, // not connected — must not appear in the enum
            },
            integration("notion", "Read and write pages."),
        ];
        let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
        let delegate_tool = tools
            .iter()
            .find(|t| t.name() == "delegate_to_integrations_agent")
            .expect(
                "collapsed delegation tool must exist when at least one integration is connected",
            );
        let desc = delegate_tool.description();
        assert!(desc.contains("gmail"));
        assert!(desc.contains("notion"));
        assert!(
            !desc.contains("github"),
            "unconnected github must not leak into the delegation tool description"
        );

        let schema = delegate_tool.parameters_schema();
        let enum_vals = schema["properties"]["toolkit"]["enum"]
            .as_array()
            .expect("toolkit enum must be present");
        let slugs: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(slugs, vec!["gmail", "notion"]);
    }

    /// Quirky toolkit slugs (dashes, mixed case) must be canonicalised
    /// before they land in the collapsed tool's enum so the
    /// LLM-provided argument can be matched with `==` rather than a
    /// fuzzy comparison.
    #[test]
    fn collapsed_tool_enum_uses_sanitised_slugs() {
        let mut orch = def("orchestrator", "t", None);
        orch.subagents = vec![SubagentEntry::Skills(SkillsWildcard { skills: "*".into() })];
        let reg = registry_with_targets();
        let integrations = vec![
            integration("Google-Calendar", "Calendar."),
            integration("Slack.Bot", "Chat."),
        ];
        let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
        let delegate_tool = tools
            .iter()
            .find(|t| t.name() == "delegate_to_integrations_agent")
            .expect("collapsed tool present");
        let schema = delegate_tool.parameters_schema();
        let enum_vals = schema["properties"]["toolkit"]["enum"].as_array().unwrap();
        let slugs: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(slugs, vec!["google_calendar", "slack_bot"]);
    }

    /// Two upstream toolkits whose names sanitise to the same slug
    /// must not silently both land in the collapsed enum — the second
    /// arrival is dropped (with a warn log) so the orchestrator's
    /// routing handle stays unambiguous. Without this guard,
    /// `Slack.Bot` and `Slack-Bot` would both render as `slack_bot`
    /// in the enum and the orchestrator could no longer distinguish
    /// them.
    /// An integration with an empty description must not render as a
    /// bare ` - slug` line in the collapsed tool description — the
    /// orchestrator LLM would have no signal about what the toolkit
    /// does. The synthesiser falls back to a generic descriptive
    /// phrase keyed on the raw toolkit name.
    #[test]
    fn empty_integration_description_falls_back_to_generic_label() {
        let mut orch = def("orchestrator", "t", None);
        orch.subagents = vec![SubagentEntry::Skills(SkillsWildcard { skills: "*".into() })];
        let reg = registry_with_targets();
        let integrations = vec![
            ConnectedIntegration {
                toolkit: "Brand.New".into(),
                description: "   ".into(),
                tools: vec![],
                connected: true,
            },
            integration("gmail", "Email."),
        ];
        let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
        let delegate_tool = tools
            .iter()
            .find(|t| t.name() == "delegate_to_integrations_agent")
            .expect("collapsed tool present");
        let desc = delegate_tool.description();
        assert!(
            desc.contains("External integration via Brand.New"),
            "expected fallback phrasing, got: {desc}"
        );
        assert!(desc.contains("Email."));
    }

    #[test]
    fn duplicate_sanitised_slug_drops_later_collisions() {
        let mut orch = def("orchestrator", "t", None);
        orch.subagents = vec![SubagentEntry::Skills(SkillsWildcard { skills: "*".into() })];
        let reg = registry_with_targets();
        let integrations = vec![
            integration("Slack.Bot", "First slack."),
            integration("Slack-Bot", "Second slack — must be dropped."),
            integration("Notion", "Pages."),
        ];
        let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
        let delegate_tool = tools
            .iter()
            .find(|t| t.name() == "delegate_to_integrations_agent")
            .expect("collapsed tool present");
        let schema = delegate_tool.parameters_schema();
        let enum_vals = schema["properties"]["toolkit"]["enum"].as_array().unwrap();
        let slugs: Vec<&str> = enum_vals.iter().map(|v| v.as_str().unwrap()).collect();
        assert_eq!(
            slugs,
            vec!["slack_bot", "notion"],
            "second slack_bot collision must be dropped, not silently shadowed"
        );
        // The dropped description must not appear in the tool description
        // either — otherwise the orchestrator would think there's a route
        // it can't actually distinguish.
        let desc = delegate_tool.description();
        assert!(desc.contains("First slack."));
        assert!(!desc.contains("Second slack"));
    }
}
