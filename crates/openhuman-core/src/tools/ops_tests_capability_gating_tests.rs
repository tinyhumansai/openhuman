use super::*;

// --- DomainSet tool classifier (#4796) ----------------------------------

#[test]
fn tool_group_classifies_gate_and_harness_families() {
    use crate::core::all::DomainGroup;

    // Gate families → their gate group (dropped under harness()).
    assert_eq!(tool_group("wallet_status"), DomainGroup::Web3);
    assert_eq!(tool_group("web3_swap_quote"), DomainGroup::Web3);
    assert_eq!(tool_group("x402_request"), DomainGroup::Web3);
    assert_eq!(tool_group("mcp_registry_search"), DomainGroup::Mcp);
    assert_eq!(tool_group("mcp_call_tool"), DomainGroup::Mcp);
    assert_eq!(tool_group("run_workflow"), DomainGroup::Skills);
    assert_eq!(tool_group("skill_registry_browse"), DomainGroup::Skills);
    assert_eq!(tool_group("list_workflows"), DomainGroup::Skills);
    // Flows has no name prefix, so EVERY flow-owned tool must be classified
    // explicitly — a missing one falls through to Platform and stays callable
    // when the Flows domain is runtime-gated off (#4797 maintainer review).
    // This list mirrors the compile-time `#[cfg(feature = "flows")]`
    // registrations and `default_tools_omits_flows_tools_when_feature_off`.
    for flow_tool in [
        "propose_workflow",
        "revise_workflow",
        "edit_workflow",
        "validate_workflow",
        "get_flow_history",
        "dry_run_workflow",
        "save_workflow",
        "suggest_workflows",
        "run_flow",
        "list_flow_runs",
        "resume_flow_run",
        "cancel_flow_run",
        "create_workflow",
        "duplicate_flow",
        "list_flows",
        "get_flow",
        "get_flow_run",
        "list_flow_connections",
        "search_tool_catalog",
        "get_tool_contract",
        "get_tool_output_sample",
        "list_agent_definitions",
        "list_connectable_toolkits",
        "list_node_kinds",
        "get_node_kind_contract",
        "flow_memory_recall",
        "flow_memory_remember",
    ] {
        assert_eq!(
            tool_group(flow_tool),
            DomainGroup::Flows,
            "flow-owned tool `{flow_tool}` must classify as Flows, not fall through to Platform"
        );
    }
    assert_eq!(tool_group("media_generate_image"), DomainGroup::Media);
    // Voice audio_* tools have no voice_/tts_/stt_ prefix — must be classified
    // explicitly, not fall through to Platform (#4808 review).
    assert_eq!(tool_group("audio_generate_podcast"), DomainGroup::Voice);
    assert_eq!(tool_group("audio_email_podcast"), DomainGroup::Voice);
    assert_eq!(
        tool_group("audio_generate_and_email_podcast"),
        DomainGroup::Voice
    );

    // Harness-mapped families → kept under harness().
    assert_eq!(tool_group("memory"), DomainGroup::Memory);
    assert_eq!(tool_group("goal_get"), DomainGroup::Threads);
    assert_eq!(tool_group("artifact_list"), DomainGroup::Agent);
    assert_eq!(tool_group("learning_list_facets"), DomainGroup::Agent);
    assert_eq!(tool_group("spawn_subagent"), DomainGroup::Agent);
    for name in [
        "ask_user_clarification",
        "wait",
        "wait_loop",
        "delegate",
        "todo",
        "spawn_parallel_agents",
    ] {
        assert_eq!(tool_group(name), DomainGroup::Agent);
    }
    assert_eq!(tool_group("config_snapshot"), DomainGroup::Config);
    assert_eq!(tool_group("workspace_init"), DomainGroup::Config);
    assert_eq!(tool_group("security_policy_info"), DomainGroup::Security);
    assert_eq!(tool_group("credential_list"), DomainGroup::Security);
    assert_eq!(tool_group("session_state"), DomainGroup::Security);
    assert_eq!(tool_group("oauth_list"), DomainGroup::Security);
    assert_eq!(tool_group("schedule"), DomainGroup::Automation);
    assert_eq!(tool_group("web_search_tool"), DomainGroup::Integrations);
    for name in [
        "web_search_tool",
        "web_answer_tool",
        "web_contents_tool",
        "tinyfish_search",
        "exa_get_contents",
        "gemini_agentic_search",
        "brave_news_search",
        "querit_search",
        "seltz_search",
        "searxng_search",
    ] {
        assert_eq!(tool_group(name), DomainGroup::Integrations);
    }

    // Everything else → Platform (dropped under harness()).
    assert_eq!(tool_group("shell"), DomainGroup::Platform);
    assert_eq!(tool_group("file_read"), DomainGroup::Platform);
}

#[test]
fn tool_group_gate_families_dropped_under_harness_not_full() {
    use crate::core::runtime::DomainSet;

    let full = DomainSet::full();
    let harness = DomainSet::harness();
    // Full keeps every family.
    for name in ["wallet_status", "run_workflow", "memory", "shell"] {
        assert!(full.allows(tool_group(name)), "full() keeps {name}");
    }
    // Harness keeps memory/threads, drops gate families AND platform.
    assert!(harness.allows(tool_group("memory")));
    assert!(harness.allows(tool_group("artifact_list")));
    assert!(harness.allows(tool_group("config_snapshot")));
    assert!(harness.allows(tool_group("security_policy_info")));
    assert!(!harness.allows(tool_group("wallet_status")));
    assert!(!harness.allows(tool_group("run_workflow")));
    assert!(!harness.allows(tool_group("shell")));
    // The previously-misclassified gate-family tools now drop under harness.
    assert!(!harness.allows(tool_group("audio_generate_podcast")));
}

#[test]
fn no_gate_family_tool_silently_defaults_to_platform() {
    use crate::core::all::DomainGroup;
    // #4808 maintainer review: a future tool in a prefix-gated family must NOT
    // fall through to Platform — otherwise it would stay callable under a custom
    // `DomainSet { platform: true, <family>: false }`, leaking the gated surface.
    // These synthetic names match no exact list, only the family prefix.
    for name in [
        "wallet_new_thing",
        "web3_new_thing",
        "x402_new_thing",
        "mcp_new_thing",
        "media_new_thing",
    ] {
        assert_ne!(
            tool_group(name),
            DomainGroup::Platform,
            "gate-family tool `{name}` must not silently default to Platform"
        );
    }
}

// --- #4797: `flows` compile-time gate ---------------------------------------

/// With the `flows` feature off, every flows-owned agent tool is compiled out
/// of the default registry entirely.
///
/// `SecurityPolicy::default()` is `Supervised` (not `ReadOnly`), so these
/// assertions are real ones: each tool *would* be registered at this tier if
/// the feature were on.
#[test]
#[cfg(not(feature = "flows"))]
fn default_tools_omits_flows_tools_when_feature_off() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(security);
    let names = tool_names(&tools);

    for absent in [
        "propose_workflow",
        "revise_workflow",
        "edit_workflow",
        "validate_workflow",
        "get_flow_history",
        "list_flow_runs",
        "resume_flow_run",
        "cancel_flow_run",
        "create_workflow",
        "duplicate_flow",
        "list_flows",
        "get_flow",
        "get_flow_run",
        "list_flow_connections",
        "search_tool_catalog",
        "get_tool_contract",
        "get_tool_output_sample",
        "list_agent_definitions",
        "list_connectable_toolkits",
        "list_node_kinds",
        "get_node_kind_contract",
        "dry_run_workflow",
        "run_flow",
        "save_workflow",
        "suggest_workflows",
        "flow_memory_recall",
        "flow_memory_remember",
    ] {
        assert!(
            !names.iter().any(|n| n == absent),
            "tool `{absent}` must be compiled out when the `flows` feature is off; got: {names:?}"
        );
    }
}

// ---- tool_group() drift guard ----------------------------------------------

/// Every `DomainGroup` must be a deliberate decision in [`tool_group`]: either a
/// representative tool name maps to it, or it is declared tool-less.
///
/// This is the guard that would have caught the #4808 leak by construction, and
/// it caught a live one on the way in: the `Inference` rule matched
/// `tokenjuice_` while the real tool is `juice_retrieve`, so CCR retrieval
/// was falling through to `Platform`.
///
/// The failure it prevents is silent. A family whose tools have no `tool_group`
/// rule lands in `Platform`, so those tools stay in the list under a
/// `DomainSet { platform: true, <family>: false }` — advertised to the model as
/// callable while the rest of the family is gated off — and conversely vanish
/// under `harness()`, which has `platform: false`.
///
/// Deliberately tests the FUNCTION, not a built registry: which tools a registry
/// contains depends on config flags, security tier and enabled integrations, so
/// a registry-derived assertion passes or fails for reasons unrelated to group
/// mapping. `REPRESENTATIVE` names are asserted to be real tool names by
/// `representative_tool_names_are_real` below, so this cannot rot into testing
/// strings that no longer exist.
#[test]
fn every_domain_group_is_accounted_for_in_tool_group() {
    use crate::core::all::DomainGroup;

    for g in DomainGroup::ALL {
        let representative = REPRESENTATIVE.iter().find(|(_, group)| group == g);
        let toolless = TOOL_LESS.contains(g);
        assert!(
            representative.is_some() ^ toolless,
            "{g:?} is in neither (or both) of REPRESENTATIVE / TOOL_LESS — decide \
             whether the family owns agent tools and list it in exactly one"
        );
        if let Some((name, want)) = representative {
            assert_eq!(
                tool_group(name),
                *want,
                "`{name}` must map to {want:?}; if it now maps elsewhere the \
                 `tool_group` rule for this family has drifted"
            );
        }
    }
}

/// The `memory` tool is registered only while memory has a usable engine: a
/// signed-out, key-less core is not offered a tool that can only fail.
#[test]
fn memory_tool_is_absent_while_memory_is_off() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert!(
        !names.iter().any(|n| n == crate::memory::MEMORY_TOOL_NAME),
        "the memory tool must not be registered with memory off; got: {names:?}"
    );
    // The v1 memory surface is gone, not merely hidden.
    for removed in [
        "memory_store",
        "memory_recall",
        "memory_forget",
        "goals",
        "tool_stats",
    ] {
        assert!(
            !names.iter().any(|n| n == removed),
            "v1 tool `{removed}` must no longer be registered"
        );
    }
}

/// With an engine bound, exactly one `memory` tool is registered.
#[test]
fn memory_tool_is_registered_once_memory_is_on() {
    let tmp = TempDir::new().unwrap();
    let cfg = test_config(&tmp);
    crate::memory::test_fixtures::bind_reference(&cfg);
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec![],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::config::HttpRequestConfig::default();
    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    assert_eq!(
        names
            .iter()
            .filter(|n| *n == crate::memory::MEMORY_TOOL_NAME)
            .count(),
        1,
        "got: {names:?}"
    );
}

/// The memory tool classifies as the Memory family (kept under `harness()`),
/// and the old `memory_`/`goals_` prefixes no longer claim anything.
#[test]
fn memory_tool_classifies_as_memory_family() {
    use crate::core::all::DomainGroup;
    use crate::core::runtime::DomainSet;
    assert_eq!(tool_group("memory"), DomainGroup::Memory);
    assert!(DomainSet::harness().allows(tool_group("memory")));
    assert_eq!(tool_group("memory_store"), DomainGroup::Platform);
}

/// A narrowed DomainSet axis still keeps the ordinary platform tools.
#[tokio::test]
async fn full_domain_set_keeps_platform_tools() {
    use crate::core::runtime::context::CoreContext;
    use crate::core::runtime::DomainSet;

    let tmp = TempDir::new().unwrap();
    let ctx = CoreContext::for_test(DomainSet::full(), None);
    let names = CoreContext::scope(ctx, async { tool_names(&expansion_tools_for(&tmp)) }).await;
    for name in ["shell", "file_read", "file_write", "todo"] {
        assert!(names.iter().any(|n| n == name), "missing `{name}`");
    }
}

/// `node_exec` / `npm_exec` are absent when the managed Node runtime is
/// compiled out — absent, not present-and-erroring, so the model is never shown
/// a tool it cannot use.
#[test]
#[cfg(not(feature = "runtime-node"))]
fn default_tools_omits_node_tools_when_runtime_node_off() {
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, "http://127.0.0.1:1");
    let tools = integration_tools_for_config(&tmp, &cfg);
    let names = tool_names(&tools);
    for absent in ["node_exec", "npm_exec"] {
        assert!(
            !names.iter().any(|n| n == absent),
            "`{absent}` must not be registered with runtime-node compiled out"
        );
    }
}
