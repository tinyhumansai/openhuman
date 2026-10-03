use super::*;
use crate::agent::harness::definition::{
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
        model: ModelSpec::Inherit,
        temperature: 0.4,
        tools: ToolScope::Wildcard,
        disallowed_tools: vec![],
        skill_filter: None,
        extra_tools: vec![],
        deferred_tools: Vec::new(),
        max_iterations: 8,
        iteration_policy: Default::default(),
        max_result_chars: None,
        max_turn_output_tokens: None,
        timeout_secs: None,
        sandbox_mode: SandboxMode::None,
        background: false,
        tokenjuice_compression: crate::inference::tokenjuice::AgentTokenjuiceCompression::Auto,
        subagents: vec![],
        delegate_name: delegate_name.map(String::from),
        agent_tier: crate::agent::harness::definition::AgentTier::Worker,
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
        SubagentEntry::AgentId("critic".into()),
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
    // `critic` has no `delegate_name` override — tool name should
    // fall back to `delegate_critic`.
    reg.insert(def(
        "critic",
        "Adversarial reviewer — cross-checks claims and diffs",
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

fn integration_with_actions(
    toolkit: &str,
    description: &str,
    actions: &[&str],
) -> ConnectedIntegration {
    let mut ci = integration(toolkit, description);
    ci.tools = actions
        .iter()
        .map(|name| crate::agent::prompts::ConnectedIntegrationTool {
            name: (*name).to_string(),
            description: format!("{name} action"),
            parameters: None,
        })
        .collect();
    ci
}

/// Baseline: an orchestrator with 2 AgentId entries + a Skills
/// wildcard, against a registry that knows both targets and a
/// connected_integrations list with three toolkits, should produce
/// 2 archetype tools plus one `Deferred` action tool per connected
/// action — and no `delegate_to_integrations_agent`. One clear service
/// action is a `tool_search` and a call, never a sub-agent spawn.
#[test]
fn collects_agentid_entries_and_expands_skills_wildcard_to_deferred_actions() {
    let orch = sample_orchestrator();
    let reg = registry_with_targets();
    let integrations = vec![
        integration_with_actions(
            "gmail",
            "Send and read email via Gmail.",
            &["GMAIL_SEND_EMAIL", "GMAIL_FETCH_EMAILS"],
        ),
        integration_with_actions(
            "github",
            "Manage repos, issues, and pull requests.",
            &["GITHUB_CREATE_ISSUE"],
        ),
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
            "research",        // researcher's delegate_name override
            "delegate_critic", // critic has no delegate_name → default
            // Actions sorted by toolkit, then action name.
            "GITHUB_CREATE_ISSUE",
            "GMAIL_FETCH_EMAILS",
            "GMAIL_SEND_EMAIL",
        ],
        "skills wildcard must expand to the connected actions, not a delegation tool"
    );
    assert!(
        !names.iter().any(|name| name.starts_with("delegate_to_")),
        "no integrations delegation tool may be synthesised"
    );

    // Archetype tool descriptions come from `when_to_use`.
    let research_tool = tools.iter().find(|t| t.name() == "research").unwrap();
    assert!(
        research_tool.description().contains("crawler"),
        "delegate description is the target's when_to_use"
    );

    // Every action is `Deferred`: off the wire, reachable through
    // `tool_search`. (The archetype delegates are `Hidden` — the collapsed
    // `delegate_to` tool advertises them — so only the actions are checked.)
    for tool in tools.iter().filter(|t| t.name().starts_with("G")) {
        assert_eq!(
            tool.exposure(),
            tinytools::ToolExposure::Deferred,
            "exposure of {}",
            tool.name()
        );
    }
}

/// The synthesised set scales only with the connected *actions*, never
/// adds a per-toolkit or collapsed delegation handle.
#[test]
fn skills_wildcard_adds_no_delegation_tool_for_any_integration_count() {
    let orch = sample_orchestrator();
    let reg = registry_with_targets();

    for n in [1usize, 3, 7, 20] {
        let integrations: Vec<_> = (0..n)
            .map(|i| {
                integration_with_actions(
                    &format!("tool{i}"),
                    &format!("Toolkit number {i}."),
                    &[&format!("TOOL{i}_ACT")],
                )
            })
            .collect();
        let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
        let delegation_count = tools
            .iter()
            .filter(|t| t.name().starts_with("delegate_to_"))
            .count();
        assert_eq!(
            delegation_count, 0,
            "no integrations delegate for {n} integrations"
        );
        let action_count = tools
            .iter()
            .filter(|t| t.exposure() == tinytools::ToolExposure::Deferred)
            .count();
        assert_eq!(action_count, n, "one deferred action per connected action");
    }
}

/// An orchestrator with a Skills wildcard but no connected
/// integrations should produce zero integration tools — nothing to
/// search for, nothing to advertise.
#[test]
fn skills_wildcard_with_no_integrations_produces_no_integration_tools() {
    let orch = sample_orchestrator();
    let reg = registry_with_targets();
    let tools = collect_orchestrator_tools(&orch, &reg, &[]);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    // `spawn_worker_thread` is temporarily disabled — see #1624.
    assert_eq!(names, vec!["research", "delegate_critic"]);
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

/// Unconnected integrations contribute no actions: the orchestrator
/// must not find (and call) an action on a toolkit the user has not
/// authorised and hit a "not connected" rejection downstream.
#[test]
fn unconnected_integrations_contribute_no_actions() {
    let orch = sample_orchestrator();
    let reg = registry_with_targets();
    let integrations = vec![
        integration_with_actions("gmail", "Send and read email.", &["GMAIL_SEND_EMAIL"]),
        ConnectedIntegration {
            toolkit: "github".into(),
            description: "GitHub access.".into(),
            tools: vec![crate::agent::prompts::ConnectedIntegrationTool {
                name: "GITHUB_CREATE_ISSUE".into(),
                description: "Create an issue".into(),
                parameters: None,
            }],
            gated_tools: vec![],
            connected: false, // not connected — its actions must not appear
            connections: Vec::new(),
            non_active_status: None,
        },
        integration_with_actions("notion", "Read and write pages.", &["NOTION_CREATE_PAGE"]),
    ];
    let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"GMAIL_SEND_EMAIL"));
    assert!(names.contains(&"NOTION_CREATE_PAGE"));
    assert!(
        !names.contains(&"GITHUB_CREATE_ISSUE"),
        "unconnected github must not leak an action into the catalogue"
    );
}

/// Actions are advertised in a stable order — toolkit, then action —
/// whatever order the backend listed the connections in, because the
/// synthesised set feeds the tool specs a session freezes.
#[test]
fn deferred_actions_are_sorted_by_toolkit_then_action() {
    let mut orch = def("orchestrator", "t", None);
    orch.subagents = vec![SubagentEntry::Skills(SkillsWildcard { skills: "*".into() })];
    let reg = registry_with_targets();
    let integrations = vec![
        integration_with_actions(
            "slack",
            "Chat.",
            &["SLACK_SEND_MESSAGE", "SLACK_LIST_CHANNELS"],
        ),
        integration_with_actions("gmail", "Email.", &["GMAIL_SEND_EMAIL"]),
    ];
    let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(
        names,
        vec![
            "GMAIL_SEND_EMAIL",
            "SLACK_LIST_CHANNELS",
            "SLACK_SEND_MESSAGE"
        ]
    );
}

/// The same action slug arriving from two toolkits keeps the first
/// arrival (by sorted toolkit) so the catalogue never carries two tools
/// under one name.
#[test]
fn duplicate_action_names_keep_the_first_arrival() {
    let mut orch = def("orchestrator", "t", None);
    orch.subagents = vec![SubagentEntry::Skills(SkillsWildcard { skills: "*".into() })];
    let reg = registry_with_targets();
    let integrations = vec![
        integration_with_actions("slack", "Chat.", &["SHARED_ACTION"]),
        integration_with_actions("gmail", "Email.", &["SHARED_ACTION", "GMAIL_SEND_EMAIL"]),
    ];
    let tools = collect_orchestrator_tools(&orch, &reg, &integrations);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(names, vec!["GMAIL_SEND_EMAIL", "SHARED_ACTION"]);
}
