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
        omit_profile: true,
        omit_memory_md: true,
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        trigger_memory_agent: Default::default(),
        tokenjuice_compression:
            crate::openhuman::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: delegate_name.map(String::from),
        agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
        source: DefinitionSource::Builtin,
        graph: Default::default(),
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
        gated_tools: vec![],
        connected: true,
        connections: Vec::new(),
        non_active_status: None,
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

/// An AgentId entry whose target carries a `delegate_name` override
/// must surface that override as the synthesised tool name — the
/// orchestrator LLM sees the override, not the default
/// `delegate_<agent_id>` shape. Mirrors the existing
/// `crypto_agent → do_crypto` precedent (#1397).
#[test]
fn subagent_with_delegate_name_override_synthesises_the_override_name() {
    let mut orch = def("orchestrator", "test", None);
    orch.subagents = vec![SubagentEntry::AgentId("custom_agent".into())];
    let mut reg = registry_with_targets();
    reg.insert(def(
        "custom_agent",
        "Specialist worker for a bespoke domain.",
        Some("do_custom"),
    ));
    let tools = collect_orchestrator_tools(&orch, &reg, &[]);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        vec!["do_custom"],
        "custom_agent subagent entry must synthesise a tool named after its \
         `delegate_name` override (`do_custom`), not the default \
         `delegate_custom_agent`"
    );
    // Description must come from the target's `when_to_use` blurb so
    // the orchestrator's LLM has domain-specific routing signal.
    let tool = tools.iter().find(|t| t.name() == "do_custom").unwrap();
    assert!(
        tool.description().contains("bespoke domain"),
        "synthesised tool description must surface the target's blurb so the LLM \
        can route intents to it"
    );
}

/// An agent with a `delegate_name` override should be exposed under that
/// name, not under the default `delegate_{id}`. `crypto_agent` is the
/// standing example — the orchestrator's prompt teaches `do_crypto`, and
/// the tool-pack table keys on it, so a regression here silently breaks
/// both.
#[test]
fn a_delegate_name_override_wins_over_the_default_delegate_prefix() {
    let mut orch = def("orchestrator", "test", None);
    orch.subagents = vec![SubagentEntry::AgentId("crypto_agent".into())];
    let mut reg = registry_with_targets();
    reg.insert(def(
        "crypto_agent",
        "Crypto specialist - wallet balances, transfers, swaps, bridges, and contract calls.",
        Some("do_crypto"),
    ));
    let tools = collect_orchestrator_tools(&orch, &reg, &[]);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        vec!["do_crypto"],
        "a subagent entry must synthesise its stable delegate_name \
         (`do_crypto`), not the default `delegate_crypto_agent`"
    );
    let tool = tools.iter().find(|t| t.name() == "do_crypto").unwrap();
    assert!(
        tool.description().contains("wallet") && tool.description().contains("swaps"),
        "synthesised tool description must surface the target's routing signal"
    );
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
/// non-delegating agents (code_executor, etc.) reach this
/// path without any subagents and must not pick up stray tools.
#[test]
fn empty_subagents_produces_no_tools() {
    let orch = def("code_executor", "First agent", None);
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
            gated_tools: vec![],
            connected: false, // not connected — must not appear in the enum
            connections: Vec::new(),
            non_active_status: None,
        },
        integration("notion", "Read and write pages."),
    ];
    let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
    let delegate_tool = tools
        .iter()
        .find(|t| t.name() == "delegate_to_integrations_agent")
        .expect("collapsed delegation tool must exist when at least one integration is connected");
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
            gated_tools: vec![],
            connected: true,
            connections: Vec::new(),
            non_active_status: None,
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

/// Two upstream toolkits whose names sanitise to the same slug
/// must not silently both land in the collapsed enum — the second
/// arrival is dropped (with a warn log) so the orchestrator's
/// routing handle stays unambiguous. Without this guard,
/// `Slack.Bot` and `Slack-Bot` would both render as `slack_bot`
/// in the enum and the orchestrator could no longer distinguish
/// them.
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
    // Sorted, not arrival order: the enum is advertised in a cached prefix, so
    // its order is fixed by slug rather than by however the backend happened to
    // list the connections (see `collect_orchestrator_tools`). The collision
    // rule this test is actually about is unaffected — "first arrival keeps the
    // slug" is decided before the sort, and the description assertions below
    // are what pin which of the two Slacks won.
    assert_eq!(
        slugs,
        vec!["notion", "slack_bot"],
        "second slack_bot collision must be dropped, not silently shadowed, \
         and the surviving slugs must be advertised in sorted order"
    );
    // The dropped description must not appear in the tool description
    // either — otherwise the orchestrator would think there's a route
    // it can't actually distinguish.
    let desc = delegate_tool.description();
    assert!(desc.contains("First slack."));
    assert!(!desc.contains("Second slack"));
}
