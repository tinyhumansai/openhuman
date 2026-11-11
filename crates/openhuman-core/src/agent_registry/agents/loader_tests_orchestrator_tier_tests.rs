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
    use tinyagents_definition::SubagentEntry;
    let def = find("orchestrator");
    let listed = def.subagents.iter().any(|e| match e {
        SubagentEntry::AgentId(id) => id == "mcp_agent",
        _ => false,
    });
    assert!(!listed, "orchestrator should call MCP tools directly");
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
    use tinyagents_definition::SubagentEntry;
    let def = find("orchestrator");
    let subagents: std::collections::HashSet<&str> = def
        .subagents
        .iter()
        .filter_map(|entry| match entry {
            SubagentEntry::AgentId(id) => Some(id.as_str()),
            SubagentEntry::Skills(_) => None,
        })
        .collect();

    let expected = "task_manager_agent";
    assert!(
        subagents.contains(expected),
        "orchestrator.subagents must list `{expected}` so the routing layer can synthesize its delegate tool"
    );
}

#[test]
fn control_specialists_have_named_tools_and_are_worker_leaves() {
    use tinyagents_definition::SubagentEntry;

    {
        let expected = "task_manager_agent";
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
fn other_builtins_default_to_worker_tier() {
    for def in load_builtins().unwrap() {
        if matches!(def.id.as_str(), "orchestrator" | "flow_discovery") {
            continue;
        }
        assert_eq!(
            def.agent_tier,
            AgentTier::Worker,
            "{} should default to worker tier (only orchestrator/flow_discovery are non-worker today)",
            def.id
        );
    }
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
        tinyagents_definition::ToolScope::Named(named) => {
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
        tinyagents_definition::ToolScope::Wildcard => {
            panic!("the orchestrator keeps a named belt")
        }
    }
}
