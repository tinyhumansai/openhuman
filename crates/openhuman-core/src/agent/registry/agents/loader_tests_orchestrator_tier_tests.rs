use super::*;

/// Wallet and market actions are the `web3` skill now, not a specialist. The
/// money-safety contract the `crypto_agent` enforced has to survive the move:
/// the guide carries the read → quote → confirm → execute order, and the
/// orchestrator prompt carries the consent rule that binds before the skill
/// is ever loaded.
#[test]
fn the_web3_skill_keeps_the_crypto_safety_contract() {
    let web3 = crate::tools::toolpacks::pack("web3").expect("web3 skill exists");
    for tool in [
        "wallet_status",
        "wallet_chain_status",
        "wallet_prepare_transfer",
        "web3_swap_quote",
        "web3_swap_execute",
        "web3_bridge_execute",
        "web3_dapp_execute",
        "x402_request",
    ] {
        assert!(web3.tools.contains(&tool), "web3 skill must hold `{tool}`");
    }
    for rule in [
        "read → quote → confirm → execute",
        "no balance tool",
        "`quote_id`",
        "Never auto-retry a write",
    ] {
        assert!(
            web3.guide.contains(rule),
            "web3 guide lost the rule `{rule}`"
        );
    }
    let prompt = include_str!("orchestrator/prompt.md");
    assert!(
        prompt.contains("Explicit yes only before moving funds"),
        "the orchestrator prompt must bind money actions to explicit consent"
    );
}

/// The orchestrator uses MCP registry tools directly, without spawning a worker.
#[test]
fn orchestrator_does_not_delegate_mcp_calls() {
    use crate::agent::harness::definition::SubagentEntry;
    let def = find("orchestrator");
    let listed = def.subagents.iter().any(|e| match e {
        SubagentEntry::AgentId(id) => id == "mcp_agent",
        _ => false,
    });
    assert!(!listed, "orchestrator should call MCP tools directly");
}

/// The `mcp` gate's load-bearing safety contract (#4799).
///
/// `agent.toml` is data and can refer to a missing optional agent. The loader
/// tolerates unknown ids rather than failing the boot.
///
/// Two independent sites provide that tolerance today:
/// * `orchestrator_tools::collect_orchestrator_tools` warns + skips
///   subagent ids absent from the registry;
/// * [`validate_tier_hierarchy`] `continue`s past unknown ids instead of
///   reporting a tier error.
///
/// This test pins the second one (the boot-blocking one) from BOTH build
/// configurations, so a future "unknown subagent ids are a hard error"
/// change fails here loudly instead of silently breaking the slim build's
/// boot — the failure mode would otherwise only appear in a
/// `--no-default-features` run, which CI's `cargo check` lane cannot catch.
#[test]
fn orchestrator_tolerates_unresolvable_subagent_id() {
    let mut def = find("orchestrator");
    def.subagents.push(SubagentEntry::AgentId(
        "definitely_not_a_compiled_in_agent".into(),
    ));

    validate_tier_hierarchy(&[def])
        .expect("validate_tier_hierarchy must tolerate an unresolvable subagent id");
}

/// MCP discovery and invocation are direct; skill setup keeps its specialist
/// route, and running a skill is the orchestrator's own `run_workflow`.
#[test]
fn orchestrator_reaches_mcp_directly_and_skills_through_hand_offs() {
    let def = find("orchestrator");
    match &def.tools {
        ToolScope::Named(tools) => {
            for required in [
                "mcp_registry_status",
                "mcp_registry_list_tools",
                "mcp_registry_connect",
                "mcp_registry_tool_call",
            ] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "missing direct MCP tool {required}"
                );
            }
            assert!(!tools.iter().any(|t| t.starts_with("skill_registry_")));
            assert!(
                tools.iter().any(|t| t == "run_workflow"),
                "the orchestrator runs installed skills itself through `run_workflow`"
            );
        }
        ToolScope::Wildcard => panic!("orchestrator must have a Named tool scope"),
    }
    {
        let specialist = "skill_setup";
        assert!(
            def.subagents
                .iter()
                .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == specialist)),
            "orchestrator must list `{specialist}` so its hand-off tool is synthesised"
        );
    }
}

#[test]
fn orchestrator_subagents_include_control_specialists() {
    use crate::agent::harness::definition::SubagentEntry;
    let def = find("orchestrator");
    let subagents: std::collections::HashSet<&str> = def
        .subagents
        .iter()
        .filter_map(|entry| match entry {
            SubagentEntry::AgentId(id) => Some(id.as_str()),
            SubagentEntry::Skills(_) => None,
        })
        .collect();

    for expected in ["task_manager_agent"] {
        assert!(
            subagents.contains(expected),
            "orchestrator.subagents must list `{expected}` so the routing layer can synthesize its delegate tool"
        );
    }
}

#[test]
fn control_specialists_have_named_tools_and_are_worker_leaves() {
    use crate::agent::harness::definition::SubagentEntry;

    for expected in ["task_manager_agent"] {
        let def = find(expected);
        assert_eq!(def.agent_tier, AgentTier::Worker);
        let visible_subagents: Vec<&str> = def
            .subagents
            .iter()
            .filter_map(|entry| match entry {
                SubagentEntry::AgentId(id) => Some(id.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            visible_subagents.is_empty(),
            "{expected} must be a worker leaf"
        );
        match def.tools {
            ToolScope::Named(tools) => {
                assert!(
                    !tools.is_empty(),
                    "{expected} must have a concrete tool allowlist"
                );
                assert!(
                    tools.iter().any(|tool| tool == "ask_user_clarification"),
                    "{expected} must be able to ask for confirmation before risky writes"
                );
                assert!(
                    !tools.iter().any(|tool| tool == "shell"),
                    "{expected} must not inherit shell access"
                );
            }
            ToolScope::Wildcard => panic!("{expected} must not use wildcard tools"),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────
// Spawn-hierarchy contract
// ─────────────────────────────────────────────────────────────────────

#[test]
fn orchestrator_is_chat_tier() {
    assert_eq!(find("orchestrator").agent_tier, AgentTier::Chat);
}

#[test]
fn planner_is_reasoning_tier() {
    assert_eq!(find("planner").agent_tier, AgentTier::Reasoning);
}

#[test]
fn other_builtins_default_to_worker_tier() {
    for def in load_builtins().unwrap() {
        if matches!(
            def.id.as_str(),
            "orchestrator" | "planner" | "flow_discovery"
        ) {
            continue;
        }
        assert_eq!(
            def.agent_tier,
            AgentTier::Worker,
            "{} should default to worker tier (only orchestrator/planner/flow_discovery are non-worker today)",
            def.id
        );
    }
}

#[test]
fn builtins_pass_tier_validation() {
    // load_builtins() already calls validate_tier_hierarchy; this
    // just makes the contract a named invariant in the test suite.
    let defs = load_builtins().expect("built-ins must pass tier validation");
    validate_tier_hierarchy(&defs).expect("explicit re-check must pass");
}

#[test]
fn rejects_chat_to_chat_delegation() {
    let mut defs = load_builtins().unwrap();
    // Add a synthetic second chat agent and have the orchestrator
    // try to delegate to it.
    let mut bad_chat = find("orchestrator");
    bad_chat.id = "second_orchestrator".to_string();
    defs.push(bad_chat);
    let orch = defs.iter_mut().find(|d| d.id == "orchestrator").unwrap();
    orch.subagents
        .push(SubagentEntry::AgentId("second_orchestrator".into()));

    let err = validate_tier_hierarchy(&defs).expect_err("chat→chat must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("chat") && msg.contains("leaf"),
        "error should call out chat-tier leaf rule, got: {msg}"
    );
}

#[test]
fn rejects_reasoning_to_reasoning_delegation() {
    let mut defs = load_builtins().unwrap();
    let mut bad_reasoning = find("planner");
    bad_reasoning.id = "second_planner".to_string();
    defs.push(bad_reasoning);
    let planner = defs.iter_mut().find(|d| d.id == "planner").unwrap();
    planner
        .subagents
        .push(SubagentEntry::AgentId("second_planner".into()));

    let err = validate_tier_hierarchy(&defs).expect_err("reasoning→reasoning must be rejected");
    assert!(err.to_string().contains("reasoning"));
}

#[test]
fn rejects_worker_with_subagents() {
    let mut defs = load_builtins().unwrap();
    let worker = defs.iter_mut().find(|d| d.id == "summarizer").unwrap();
    worker
        .subagents
        .push(SubagentEntry::AgentId("critic".into()));

    let err = validate_tier_hierarchy(&defs)
        .expect_err("worker with declared subagents must be rejected");
    let msg = err.to_string();
    assert!(
        msg.contains("worker") && msg.contains("leaf"),
        "error should call out worker leaf rule, got: {msg}"
    );
}

#[test]
fn allows_skill_wildcards_on_any_non_worker_tier() {
    // Skills wildcards expand to searchable integration actions, not to
    // an agent, so there is no tier pair for the check to police.
    let mut defs = load_builtins().unwrap();
    let planner = defs.iter_mut().find(|d| d.id == "planner").unwrap();
    planner.subagents.push(SubagentEntry::Skills(
        crate::agent::harness::definition::SkillsWildcard { skills: "*".into() },
    ));
    validate_tier_hierarchy(&defs).expect("skill wildcards on reasoning tier must validate");
}

/// The orchestrator defers its rarely-used or duplicate-route tools for itself
/// only (`deferred_tools`): they stay registered, searchable through
/// `tool_search` and callable by name, while other agents that name them keep
/// them advertised. The belt must opt into discovery for the list to apply.
#[test]
fn orchestrator_defers_its_duplicate_route_tools() {
    let def = find("orchestrator");
    let mut deferred = def.deferred_tools.clone();
    deferred.sort();
    assert_eq!(
        deferred,
        vec![
            "composio_list_toolkits",
            "current_time",
            "file_read",
            "file_write",
            "http_request",
            "mcp_registry_connect",
            "mcp_registry_list_tools",
            "mcp_registry_status",
            "mcp_registry_tool_call",
        ]
    );
    match &def.tools {
        crate::agent::harness::definition::ToolScope::Named(named) => {
            assert!(
                named.iter().any(|name| name == "tool_search"),
                "`deferred_tools` only applies to a belt that opted into discovery"
            );
            for name in &def.deferred_tools {
                assert!(
                    named.contains(name),
                    "`{name}` is deferred but not on the belt, so deferring it does nothing"
                );
            }
        }
        crate::agent::harness::definition::ToolScope::Wildcard => {
            panic!("the orchestrator keeps a named belt")
        }
    }
}
