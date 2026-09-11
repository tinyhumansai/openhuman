//! Tool-pack behaviour.
//!
//! The pair of assertions that matter most are the negative ones: that a packed
//! tool's schema really is withheld, and that `use_skill` cannot be used to
//! reach a tool through the wrong skill or to launder its permission level.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use super::*;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult, ToolTimeout};

struct FakeTool {
    name: &'static str,
    level: PermissionLevel,
    external: bool,
    timeout: ToolTimeout,
}

#[async_trait]
impl Tool for FakeTool {
    fn name(&self) -> &str {
        self.name
    }
    fn description(&self) -> &str {
        "fake"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {"marker": {"type": "string"}}})
    }
    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        Ok(ToolResult::success(format!("{}:{}", self.name, args)))
    }
    fn permission_level(&self) -> PermissionLevel {
        self.level
    }
    fn external_effect_with_args(&self, _args: &Value) -> bool {
        self.external
    }
    fn timeout_policy(&self, _args: &Value) -> ToolTimeout {
        self.timeout
    }
}

/// A registry holding one real packed tool plus the two pack tools, bound.
fn registry_with(name: &'static str, level: PermissionLevel) -> Arc<Vec<Box<dyn Tool>>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(FakeTool {
        name,
        level,
        external: false,
        timeout: ToolTimeout::Inherit,
    })];
    append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&tools);
    tools
}

/// A registry whose one packed tool is externally effectful and unbounded.
fn external_unbounded_registry(name: &'static str) -> Arc<Vec<Box<dyn Tool>>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(FakeTool {
        name,
        level: PermissionLevel::ReadOnly,
        external: true,
        timeout: ToolTimeout::Unbounded,
    })];
    append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&tools);
    tools
}

fn find<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|t| t.name() == name)
        .map(AsRef::as_ref)
        .unwrap_or_else(|| panic!("{name} missing"))
}

#[test]
fn every_packed_name_belongs_to_exactly_one_pack() {
    let mut seen = HashSet::new();
    for name in all_packed_tool_names() {
        assert!(seen.insert(name), "`{name}` is claimed by two packs");
    }
}

#[test]
fn packed_names_are_withheld_and_replaced() {
    let packed = all_packed_tool_names();
    let sample = packed[0];
    let mut visible: HashSet<String> = [sample.to_string(), "file_read".to_string()]
        .into_iter()
        .collect();

    strip_packed_from_visible(&mut visible, "orchestrator");

    assert!(!visible.contains(sample), "packed tool stayed advertised");
    assert!(visible.contains("file_read"), "unpacked tool was dropped");
    assert!(visible.contains(LOAD_SKILL) && visible.contains(USE_SKILL));
}

#[test]
fn an_agent_that_lost_nothing_gains_nothing() {
    // A narrow sub-agent must not grow two tools that can only report an empty
    // skill, so the pack tools are added only when something was withheld.
    let mut visible: HashSet<String> = ["file_read".to_string()].into_iter().collect();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert_eq!(visible.len(), 1);
    assert!(!visible.contains(LOAD_SKILL));
}

#[test]
fn an_empty_visible_set_is_left_alone() {
    // Empty is the harness's "everything is visible" sentinel, not "nothing".
    let mut visible: HashSet<String> = HashSet::new();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert!(visible.is_empty());
}

#[tokio::test]
async fn load_skill_renders_the_schema_of_a_present_tool() {
    let name = pack("crypto").unwrap().tools[0];
    let tools = registry_with(name, PermissionLevel::ReadOnly);
    let result = find(&tools, LOAD_SKILL)
        .execute(json!({"skill": "crypto"}))
        .await
        .unwrap();
    assert!(!result.is_error);
    let text = format!("{:?}", result.content);
    assert!(text.contains(name), "rendered pack omitted `{name}`");
    assert!(
        text.contains("marker"),
        "rendered pack omitted the arg schema"
    );
}

#[tokio::test]
async fn load_skill_rejects_an_unknown_skill() {
    let tools = registry_with("do_crypto", PermissionLevel::ReadOnly);
    let result = find(&tools, LOAD_SKILL)
        .execute(json!({"skill": "nope"}))
        .await
        .unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn use_skill_dispatches_to_the_packed_tool() {
    let name = pack("crypto").unwrap().tools[0];
    let tools = registry_with(name, PermissionLevel::ReadOnly);
    let result = find(&tools, USE_SKILL)
        .execute(json!({"skill": "crypto", "tool": name, "args": {"marker": "x"}}))
        .await
        .unwrap();
    assert!(!result.is_error);
    let text = format!("{:?}", result.content);
    assert!(
        text.contains("marker"),
        "inner args were not forwarded: {text}"
    );
}

#[tokio::test]
async fn use_skill_refuses_a_tool_from_another_skill() {
    // Cross-skill dispatch would make the `skill` argument decoration and let a
    // workflow skill reach a crypto write.
    let crypto = pack("crypto").unwrap().tools[0];
    let tools = registry_with(crypto, PermissionLevel::Dangerous);
    let result = find(&tools, USE_SKILL)
        .execute(json!({"skill": "workflows", "tool": crypto, "args": {}}))
        .await
        .unwrap();
    assert!(result.is_error, "cross-skill dispatch was admitted");
}

#[test]
fn use_skill_reports_the_inner_tools_permission_level() {
    // The harness gates on this. Reporting the proxy's own level would launder
    // a dangerous packed tool onto a channel that refuses it.
    let name = pack("crypto").unwrap().tools[0];
    let tools = registry_with(name, PermissionLevel::Dangerous);
    let use_skill = find(&tools, USE_SKILL);
    assert_eq!(
        use_skill.permission_level_with_args(&json!({"skill": "crypto", "tool": name})),
        PermissionLevel::Dangerous
    );
}

#[test]
fn use_skill_forwards_the_inner_tools_external_effect() {
    // The approval gate calls external_effect_with_args on the proxy; a proxy
    // that reported false would let an effectful packed tool skip the prompt.
    let name = pack("crypto").unwrap().tools[0];
    let tools = external_unbounded_registry(name);
    let use_skill = find(&tools, USE_SKILL);
    assert!(
        use_skill.external_effect_with_args(&json!({"skill": "crypto", "tool": name})),
        "proxy must forward the inner tool's external-effect classification"
    );
}

#[test]
fn use_skill_forwards_the_inner_tools_timeout_policy() {
    // A packed scripting tool must run under its own deadline, not the proxy's.
    let name = pack("crypto").unwrap().tools[0];
    let tools = external_unbounded_registry(name);
    let use_skill = find(&tools, USE_SKILL);
    assert_eq!(
        use_skill.timeout_policy(&json!({"skill": "crypto", "tool": name})),
        ToolTimeout::Unbounded
    );
}

#[test]
fn an_unresolvable_call_reports_inherit_timeout() {
    let name = pack("crypto").unwrap().tools[0];
    let tools = external_unbounded_registry(name);
    let use_skill = find(&tools, USE_SKILL);
    assert_eq!(
        use_skill.timeout_policy(&json!({"skill": "crypto", "tool": "nonexistent"})),
        ToolTimeout::Inherit
    );
}

#[test]
fn an_unresolvable_call_reports_the_ceiling_not_a_permissive_default() {
    let name = pack("crypto").unwrap().tools[0];
    let tools = registry_with(name, PermissionLevel::Dangerous);
    let use_skill = find(&tools, USE_SKILL);
    assert_eq!(
        use_skill.permission_level_with_args(&json!({"skill": "crypto", "tool": "nonexistent"})),
        PermissionLevel::Dangerous
    );
}

#[test]
fn an_unbound_handle_degrades_closed() {
    // A pack tool that never got bound must not become a permissive passthrough.
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    append_pack_tools(&mut tools);
    let use_skill = find(&tools, USE_SKILL);
    assert_eq!(use_skill.permission_level(), PermissionLevel::Dangerous);
}

#[test]
fn every_pack_declares_the_tools_it_is_named_for() {
    // Membership is compiled-in data, so a typo here is invisible until a
    // `load_skill` at runtime renders a pack that withheld nothing. Pin the
    // exact set per pack rather than a count.
    let expect: &[(&str, &[&str])] = &[
        (
            "workflows",
            &[
                "build_workflow",
                "discover_workflows",
                "run_workflow",
                "await_workflow",
                "describe_workflow",
                "list_workflows",
                "list_workflow_runs",
                "read_workflow_run_log",
                "propose_workflow",
                "revise_workflow",
                "edit_workflow",
                "validate_workflow",
                "save_workflow",
                "create_workflow",
                "duplicate_flow",
                "dry_run_workflow",
                "list_flows",
                "get_flow",
                "get_flow_history",
                "get_flow_run",
                "list_flow_runs",
                "list_flow_connections",
                "cancel_flow_run",
                "resume_flow_run",
                "suggest_workflows",
                "search_tool_catalog",
                "get_tool_contract",
                "get_tool_output_sample",
                "list_node_kinds",
                "get_node_kind_contract",
                "list_agent_profiles",
                "list_connectable_toolkits",
            ],
        ),
        (
            "crypto",
            &[
                "do_crypto",
                "wallet_status",
                "wallet_balances",
                "wallet_network_defaults",
                "wallet_supported_assets",
                "wallet_chain_status",
                "wallet_encode_erc20_transfer",
                "wallet_prepare_transfer",
                "wallet_execute_prepared",
                "wallet_tx_status",
                "wallet_tx_receipt",
                "wallet_lookup_tx",
                "web3_swap_routes",
                "web3_swap_quote",
                "web3_swap_execute",
                "web3_bridge_quote",
                "web3_bridge_execute",
                "web3_dapp_call",
                "web3_dapp_execute",
                "x402_request",
            ],
        ),
        (
            "integrations",
            &[
                "use_mcp_server",
                "setup_mcp_server",
                "mcp_registry_status",
                "mcp_registry_search",
                "mcp_registry_get",
                "mcp_registry_installed_list",
                "mcp_registry_list_tools",
                "mcp_registry_connect",
                "mcp_registry_disconnect",
                "mcp_registry_tool_call",
                "mcp_registry_config_assist",
                "mcp_registry_install",
                "mcp_registry_uninstall",
            ],
        ),
        (
            "composio",
            &[
                "composio",
                "composio_authorize",
                "composio_connect",
                "composio_execute",
                "composio_list_connections",
                "composio_list_toolkits",
                "composio_list_tools",
            ],
        ),
        (
            "skills",
            &[
                "run_skill",
                "setup_skills",
                "skill_registry_browse",
                "skill_registry_search",
                "skill_registry_install",
                "skill_registry_sources",
                "skill_registry_uninstall",
                "skill_runtime_resolve_runtimes",
                "install_workflow_from_url",
                "uninstall_workflow",
                "read_workflow_resource",
            ],
        ),
        ("documents", &["generate_document", "generate_presentation"]),
        (
            "audio",
            &[
                "audio_generate_podcast",
                "audio_email_podcast",
                "audio_generate_and_email_podcast",
            ],
        ),
        (
            "system",
            &[
                "config_snapshot",
                "config_get_client_config",
                "config_get_autonomy",
                "config_get_search",
                "config_get_runtime_flags",
                "config_resolve_api_url",
                "config_get_data_paths",
                "doctor_health",
                "doctor_models",
                "health_snapshot",
                "health_system_info",
                "dashboard_model_health",
                "cost_get_dashboard",
                "cost_get_daily_history",
                "cost_get_summary",
                "security_policy_info",
                "service_status",
                "service_start",
                "service_stop",
                "service_restart",
                "service_shutdown",
                "service_install",
                "service_uninstall",
                "daemon_host_prefs_get",
                "daemon_host_prefs_set",
                "proxy_config",
            ],
        ),
        ("goals", &["goal_set", "goal_get", "goal_complete"]),
        ("app_update", &["update_check", "update_apply"]),
    ];

    for (id, tools) in expect {
        let found = registry::pack(id).unwrap_or_else(|| panic!("pack `{id}` is missing"));
        assert_eq!(found.tools, *tools, "pack `{id}` membership drifted");
    }

    assert_eq!(
        registry::PACKS.len(),
        expect.len(),
        "a pack was added without pinning its membership here"
    );
}

#[test]
fn a_packs_owner_keeps_its_belt_advertised() {
    // `settings_agent` IS the system family. Withholding its own belt would
    // buy a `load_skill` round trip per turn and hide nothing that is idle.
    let mut visible: HashSet<String> = ["doctor_health".to_string(), "file_read".to_string()]
        .into_iter()
        .collect();
    strip_packed_from_visible(&mut visible, "settings_agent");
    assert!(
        visible.contains("doctor_health"),
        "the system pack's owner lost its own tool"
    );
    assert!(!visible.contains(LOAD_SKILL), "owner gained pack tools");
}

#[test]
fn a_packs_owner_still_loses_every_other_pack() {
    // Ownership is per pack, not a blanket exemption: `settings_agent` owns
    // `system` and `app_update`, and must still lose `crypto`.
    let mut visible: HashSet<String> = ["doctor_health".to_string(), "wallet_status".to_string()]
        .into_iter()
        .collect();
    strip_packed_from_visible(&mut visible, "settings_agent");
    assert!(visible.contains("doctor_health"));
    assert!(
        !visible.contains("wallet_status"),
        "non-owned pack survived"
    );
    assert!(visible.contains(LOAD_SKILL) && visible.contains(USE_SKILL));
}

#[test]
fn every_owner_names_a_pack_tool_it_actually_declares() {
    // An owner id that no longer matches an agent (renamed, deleted) silently
    // stops exempting anything. There is no agent registry in this unit's
    // scope, so pin the weaker invariant that matters here: no pack lists an
    // owner twice, and no pack claims an owner while owning no tools.
    for pack in registry::PACKS {
        let mut seen = HashSet::new();
        for owner in pack.owners {
            assert!(
                seen.insert(owner),
                "pack `{}` lists owner `{owner}` twice",
                pack.id
            );
        }
        assert!(
            pack.owners.is_empty() || !pack.tools.is_empty(),
            "pack `{}` has owners but no tools",
            pack.id
        );
    }
}

#[test]
fn the_reactive_fleet_tools_are_never_packed() {
    // Packing these would put a `load_skill` round-trip between an async
    // worker returning and the parent being able to steer or collect it.
    // See `DELIBERATELY_UNPACKED_FLEET_TOOLS` for the full reasoning.
    for name in registry::DELIBERATELY_UNPACKED_FLEET_TOOLS {
        assert!(
            registry::pack_for_tool(name).is_none(),
            "`{name}` is needed reactively mid-turn and must stay advertised"
        );
    }
}

#[test]
fn rebinding_a_pack_handle_repoints_it_at_the_new_registry() {
    // `bind_pack_registry`'s own docs say to "call this after **every**
    // rebinding of the agent's tool `Arc`", but the handle used to hold a
    // `OnceLock`, so the second write was dropped on the floor. An agent that
    // rebuilt its tool vector kept a `Weak` into the old allocation; once that
    // allocation went away the upgrade failed and every `load_skill` /
    // `use_skill` reported the registry as unavailable for the rest of the
    // session. Last write must win.
    let name = pack("crypto").unwrap().tools[0];

    // The agent's first tool `Arc`, with the packed tool marked Dangerous.
    let first = registry_with(name, PermissionLevel::Dangerous);
    let use_skill = find(&first, USE_SKILL);
    let args = json!({"skill": "crypto", "tool": name});
    assert_eq!(
        use_skill.permission_level_with_args(&args),
        PermissionLevel::Dangerous,
        "sanity: the first binding resolves"
    );

    // The agent rebuilds its registry into a *different* allocation, where the
    // same packed tool is only ReadOnly.
    let mut rebuilt: Vec<Box<dyn Tool>> = vec![Box::new(FakeTool {
        name,
        level: PermissionLevel::ReadOnly,
        external: false,
        timeout: ToolTimeout::Inherit,
    })];
    append_pack_tools(&mut rebuilt);
    let rebuilt = Arc::new(rebuilt);

    // Re-point the ORIGINAL handle at it, which is what a rebuild does.
    crate::openhuman::tools::traits::pack_registry_handle(use_skill)
        .expect("use_skill exposes a pack registry handle")
        .bind(Arc::downgrade(&rebuilt));

    assert_eq!(
        use_skill.permission_level_with_args(&args),
        PermissionLevel::ReadOnly,
        "the rebound handle must resolve against the new registry, not the old one"
    );
}
