use super::*;

/// Crypto Agent (#1397) is the dedicated specialist for wallet
/// actions and market operations. It must have a *narrow* tool
/// allowlist (no shell, no file_write, no broad HTTP), MUST keep
/// the safety preamble on (financial-risk gate), and MUST require
/// quote/confirm-before-execute via `ask_user_clarification`.
#[test]
fn crypto_agent_has_narrow_wallet_market_tools_and_safety_on() {
    let def = find("crypto_agent");
    // Hint must be burst — latency matters for the narrow quote/execute
    // workflow and provider routing still preserves explicit agentic BYOK.
    assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "burst"));
    assert_eq!(def.sandbox_mode, SandboxMode::None);
    // Financial-risk agent — global safety preamble stays ON.
    assert!(
        !def.omit_safety_preamble,
        "crypto_agent must keep the global safety preamble — financial-risk gate"
    );
    match &def.tools {
        ToolScope::Named(tools) => {
            // Wallet read surface.
            // Only names with a registered agent Tool. `wallet_balances`,
            // `wallet_network_defaults`, `wallet_supported_assets` and
            // `wallet_encode_erc20_transfer` are `wallet.*` RPC methods with no
            // Tool wrapper — asserting them here pinned an allowlist entry the
            // spawn filter drops, so the assertion passed while the capability
            // did not exist.
            for required in ["wallet_status", "wallet_chain_status"] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "crypto_agent needs read tool `{required}`"
                );
            }
            // Quote / prepare surface: native+token transfers on the
            // wallet, swaps/bridges/dapp calls on the web3 layer.
            for required in [
                "wallet_prepare_transfer",
                "web3_swap_quote",
                "web3_bridge_quote",
                "web3_dapp_call",
            ] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "crypto_agent needs prepare tool `{required}`"
                );
            }
            // Transaction inspection surface.
            for required in ["wallet_tx_status", "wallet_tx_receipt", "wallet_lookup_tx"] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "crypto_agent needs tx-read tool `{required}`"
                );
            }
            // Execute surface. There is no `wallet_execute_prepared` TOOL —
            // it is a `wallet.*` RPC only — so a plain transfer is prepare-only
            // for this agent. The executable surface is the web3 quote/execute
            // family, and THAT is what the prompt's confirm-before-execute rule
            // now gates on.
            assert!(
                !tools.iter().any(|t| t == "wallet_execute_prepared"),
                "wallet_execute_prepared has no Tool wrapper; listing it puts a \
                 call in the prompt that can never resolve"
            );
            for required in [
                "web3_swap_execute",
                "web3_bridge_execute",
                "web3_dapp_execute",
            ] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "crypto_agent needs {required} for the confirm-before-execute flow"
                );
            }
            // Confirmation gate — MUST be present so the prompt's
            // "confirm before execute" rule is mechanically enforceable.
            assert!(
                tools.iter().any(|t| t == "ask_user_clarification"),
                "crypto_agent needs ask_user_clarification to gate write ops"
            );
            // Market grounding + time helpers. Memory retrieval is the
            // orchestrator's on-demand concern — this specialist gets a
            // grounded request and does not pre-fetch memory itself.
            for required in [
                "stock_quote",
                "stock_exchange_rate",
                "stock_crypto_series",
                "current_time",
            ] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "crypto_agent needs supporting tool `{required}`"
                );
            }
            // x402 paid HTTP requests — signs on-chain USDC payments
            // for APIs behind HTTP 402 challenges.
            assert!(
                tools.iter().any(|t| t == "x402_request"),
                "crypto_agent needs x402_request for paid API access"
            );
            assert!(!tools.iter().any(|t| t == "call_memory_agent"));
            // Hard exclusions — no broad-surface or write-anywhere tools.
            // Includes the orchestrator-level delegate_* tools so a future
            // TOML edit can't accidentally hand crypto writes to the
            // generic integrations or code-execution paths.
            for forbidden in [
                "shell",
                "file_write",
                "curl",
                "http_request",
                "composio_execute",
                "composio_list_tools",
                "spawn_subagent",
                "spawn_worker_thread",
                "delegate_to_integrations_agent",
                // Synthesised delegation tools use the unprefixed
                // `delegate_name` overrides — forbid those names too.
                "run_code",
                "research",
                "plan",
            ] {
                assert!(
                    !tools.iter().any(|t| t == forbidden),
                    "crypto_agent must NOT have `{forbidden}` — keeps blast radius bounded"
                );
            }
        }
        ToolScope::Wildcard => panic!("crypto_agent must have a Named tool scope"),
    }
    // Keep iteration cap tight — quote → confirm → execute is a
    // 3-step loop, not a research crawl.
    assert!(
        def.max_iterations <= 10,
        "crypto_agent max_iterations must stay tight (got {})",
        def.max_iterations
    );
    assert!(def.omit_identity);
    assert!(def.omit_memory_context);
    // Pure-function specialist (omit_memory_context = true) — no eager
    // memory pre-fetch; the orchestrator hands it a grounded request.
    assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
}

/// Routing: the orchestrator must list `crypto_agent` in its
/// `subagents` so a `delegate_do_crypto` tool is synthesised at
/// agent-build time. Without this entry the orchestrator can't
/// route crypto-shaped requests to the specialist.
#[test]
fn orchestrator_subagents_include_crypto_agent() {
    use crate::openhuman::agent::harness::definition::SubagentEntry;
    let def = find("orchestrator");
    let listed = def.subagents.iter().any(|e| match e {
        SubagentEntry::AgentId(id) => id == "crypto_agent",
        _ => false,
    });
    assert!(
        listed,
        "orchestrator.subagents must list `crypto_agent` so the \
         routing layer can synthesise `delegate_do_crypto`"
    );
}

/// Routing: the orchestrator must list `mcp_agent` in its `subagents`
/// so a `delegate_use_mcp_server` tool is synthesised at agent-build
/// time. Without this entry the orchestrator can only *set up* MCP
/// servers (via `mcp_setup`) and has no route to actually *use* an
/// already-connected server's tools from chat (issue #3495).
#[test]
fn orchestrator_subagents_include_mcp_agent() {
    use crate::openhuman::agent::harness::definition::SubagentEntry;
    let def = find("orchestrator");
    let listed = def.subagents.iter().any(|e| match e {
        SubagentEntry::AgentId(id) => id == "mcp_agent",
        _ => false,
    });
    assert!(
        listed,
        "orchestrator.subagents must list `mcp_agent` so the routing \
         layer can synthesise `delegate_use_mcp_server`"
    );
}

/// The `mcp` gate's load-bearing safety contract (#4799).
///
/// `agent.toml` is DATA — it cannot be `#[cfg]`'d, so the orchestrator goes
/// on listing `mcp_agent` in `subagents` even in builds where the `mcp`
/// feature dropped `mcp_agent` from [`BUILTINS`]. That leaves a subagent id
/// that resolves to nothing, and the whole gate rests on the loader
/// TOLERATING it rather than failing the boot.
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

    validate_tier_hierarchy(&[def]).expect(
        "validate_tier_hierarchy must tolerate an unresolvable subagent id — the `mcp` \
         feature gate relies on it (orchestrator's agent.toml lists `mcp_agent` even in \
         builds that compile `mcp_agent` out)",
    );
}

/// Companion to the above, asserting the real gated shape rather than a
/// synthetic id: with `mcp` compiled out, `mcp_agent` is genuinely absent
/// from the loaded set while the orchestrator still lists it — and
/// `load_builtins` (which runs `validate_tier_hierarchy` internally) must
/// still succeed, i.e. the core boots.
#[test]
#[cfg(not(feature = "mcp"))]
fn orchestrator_tolerates_absent_mcp_agent() {
    let defs = load_builtins().expect(
        "load_builtins must succeed with `mcp` compiled out — the orchestrator's dangling \
         `mcp_agent` subagent reference must not fail the boot",
    );

    assert!(
        !defs.iter().any(|d| d.id == "mcp_agent"),
        "`mcp_agent` must be compiled out when the `mcp` feature is off"
    );

    let orchestrator = defs
        .iter()
        .find(|d| d.id == "orchestrator")
        .expect("orchestrator must still load");
    assert!(
        orchestrator.subagents.iter().any(|e| matches!(
            e,
            SubagentEntry::AgentId(id) if id == "mcp_agent"
        )),
        "orchestrator.agent.toml is data and still lists `mcp_agent` — this dangling \
         reference is exactly what the loader must tolerate"
    );
}

/// The orchestrator gets lightweight MCP discovery (`mcp_registry_status`,
/// like `composio_list_connections`) but must NOT carry the per-server
/// enumerate/execute tools — those belong to `mcp_agent`, keeping the
/// chat agent's schema from ballooning with every connected server's
/// full toolset (#3495).
#[test]
fn orchestrator_has_mcp_discovery_but_not_execution() {
    let def = find("orchestrator");
    match &def.tools {
        ToolScope::Named(tools) => {
            assert!(
                tools.iter().any(|t| t == "mcp_registry_status"),
                "orchestrator must have mcp_registry_status for lightweight MCP discovery"
            );
            for forbidden in ["mcp_registry_list_tools", "mcp_registry_tool_call"] {
                assert!(
                    !tools.iter().any(|t| t == forbidden),
                    "orchestrator must NOT have `{forbidden}` — enumerating/calling \
                     connected MCP tools is mcp_agent's job (keeps the chat schema small)"
                );
            }
        }
        ToolScope::Wildcard => panic!("orchestrator must have a Named tool scope"),
    }
}

/// `mcp_agent` is the connected-server execution specialist: it must hold
/// the discover + call surface and a stable `use_mcp_server` delegate name,
/// but must NOT hold the secret-handling install/uninstall tools (those are
/// `mcp_setup`'s) or any shell/file/network capability.
///
/// Gated: `find` panics on a missing id, and the `mcp` feature drops
/// `mcp_agent` from [`BUILTINS`] entirely.
#[test]
#[cfg(feature = "mcp")]
fn mcp_agent_drives_connected_servers_without_install_or_shell() {
    let def = find("mcp_agent");
    assert_eq!(def.agent_tier, AgentTier::Worker);
    assert_eq!(
        def.delegate_name.as_deref(),
        Some("use_mcp_server"),
        "mcp_agent must keep its `use_mcp_server` delegate name stable"
    );
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
                    "mcp_agent missing `{required}`"
                );
            }
            for forbidden in [
                "mcp_registry_install",
                "mcp_registry_uninstall",
                "shell",
                "file_write",
                "curl",
                "http_request",
            ] {
                assert!(
                    !tools.iter().any(|t| t == forbidden),
                    "mcp_agent must NOT have `{forbidden}` — it only relays through \
                     already-connected servers; install/secrets belong to mcp_setup"
                );
            }
        }
        ToolScope::Wildcard => panic!("mcp_agent must have a Named tool scope"),
    }
}

#[test]
fn orchestrator_subagents_include_skill_creator() {
    use crate::openhuman::agent::harness::definition::SubagentEntry;
    let def = find("orchestrator");
    let listed = def.subagents.iter().any(|e| match e {
        SubagentEntry::AgentId(id) => id == "skill_creator",
        _ => false,
    });
    assert!(
        listed,
        "orchestrator.subagents must list `skill_creator` so the \
        routing layer can synthesise `create_skill`"
    );
}

#[test]
fn orchestrator_subagents_include_control_specialists() {
    use crate::openhuman::agent::harness::definition::SubagentEntry;
    let def = find("orchestrator");
    let subagents: std::collections::HashSet<&str> = def
        .subagents
        .iter()
        .filter_map(|entry| match entry {
            SubagentEntry::AgentId(id) => Some(id.as_str()),
            SubagentEntry::Skills(_) => None,
        })
        .collect();

    for expected in [
        "task_manager_agent",
        "settings_agent",
        "profile_memory_agent",
    ] {
        assert!(
            subagents.contains(expected),
            "orchestrator.subagents must list `{expected}` so the routing layer can synthesize its delegate tool"
        );
    }
}

#[test]
fn control_specialists_have_named_tools_and_are_worker_leaves() {
    use crate::openhuman::agent::harness::definition::SubagentEntry;

    for expected in [
        "task_manager_agent",
        "settings_agent",
        "profile_memory_agent",
    ] {
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
            "orchestrator" | "planner" | "subconscious" | "flow_discovery"
        ) {
            continue;
        }
        assert_eq!(
            def.agent_tier,
            AgentTier::Worker,
            "{} should default to worker tier (only orchestrator/planner/subconscious/flow_discovery are non-worker today)",
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
    let researcher = defs.iter_mut().find(|d| d.id == "researcher").unwrap();
    researcher
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
    // Skills wildcards collapse to delegate_to_integrations_agent
    // and must not be policed by the tier check (it'd be a false
    // positive — they fan out to a worker anyway).
    let mut defs = load_builtins().unwrap();
    let planner = defs.iter_mut().find(|d| d.id == "planner").unwrap();
    planner.subagents.push(SubagentEntry::Skills(
        crate::openhuman::agent::harness::definition::SkillsWildcard { skills: "*".into() },
    ));
    validate_tier_hierarchy(&defs).expect("skill wildcards on reasoning tier must validate");
}
