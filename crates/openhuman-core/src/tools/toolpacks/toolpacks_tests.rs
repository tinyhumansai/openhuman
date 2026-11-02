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
use tinyagents_harness_tools::skills::USE_SKILL;
use tinytools::{PermissionLevel, Tool, ToolResult, ToolTimeout};

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

/// A registry holding several real packed tools plus the two pack tools, bound.
fn registry_with_all(
    names: &[&'static str],
) -> (
    Arc<Vec<Box<dyn Tool>>>,
    tinyagents_harness_tools::skills::PackRegistryHandle,
) {
    let mut tools: Vec<Box<dyn Tool>> = names
        .iter()
        .map(|name| {
            Box::new(FakeTool {
                name,
                level: PermissionLevel::ReadOnly,
                external: false,
                timeout: ToolTimeout::Inherit,
            }) as Box<dyn Tool>
        })
        .collect();
    let handle = append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&handle, &tools);
    (tools, handle)
}

/// A registry holding one real packed tool plus the two pack tools, bound.
fn registry_with(
    name: &'static str,
    level: PermissionLevel,
) -> (
    Arc<Vec<Box<dyn Tool>>>,
    tinyagents_harness_tools::skills::PackRegistryHandle,
) {
    let mut tools: Vec<Box<dyn Tool>> = vec![Box::new(FakeTool {
        name,
        level,
        external: false,
        timeout: ToolTimeout::Inherit,
    })];
    let handle = append_pack_tools(&mut tools);
    let tools = Arc::new(tools);
    bind_pack_registry(&handle, &tools);
    (tools, handle)
}

fn find<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|t| t.name() == name)
        .map(AsRef::as_ref)
        .unwrap_or_else(|| panic!("{name} missing"))
}

/// Closing a goal must stay directly callable.
///
/// `goal_complete` is the one goal operation an agent reaches for reactively —
/// at the end of work it has just finished. Packing it would put a `use_skill`
/// round trip at exactly that moment, and the failure when the model does not
/// pay it is silent: the objective stays open and keeps driving autonomous
/// continuation. Everything else about goals is behind the `goals` pack
/// precisely so this one tool is cheap to keep visible.
#[test]
fn closing_a_goal_is_never_packed() {
    assert!(
        !all_packed_tool_names().contains(&"goal_complete"),
        "`goal_complete` was packed; see the carve-out note on the `goals` pack"
    );
    // And the rest of the family is, or the carve-out saved nothing.
    for held in ["goal_get", "goal_set"] {
        assert!(
            all_packed_tool_names().contains(&held),
            "`{held}` should be reachable through the `goals` pack, not on the wire"
        );
    }
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
    let mut visible: HashSet<String> = [sample.to_string(), "shell".to_string()]
        .into_iter()
        .collect();

    strip_packed_from_visible(&mut visible, "unrelated_agent");

    assert!(!visible.contains(sample), "packed tool stayed advertised");
    assert!(visible.contains("shell"), "unpacked tool was dropped");
    assert!(visible.contains(USE_SKILL));
}

#[test]
fn an_agent_that_lost_nothing_gains_nothing() {
    // A narrow sub-agent must not grow a tool that can only report an empty
    // skill, so the pack tool is added only when something was withheld.
    let mut visible: HashSet<String> = ["shell".to_string()].into_iter().collect();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert_eq!(visible.len(), 1);
    assert!(!visible.contains(USE_SKILL));
}

#[test]
fn an_empty_visible_set_is_left_alone() {
    // Empty is the harness's "everything is visible" sentinel, not "nothing".
    let mut visible: HashSet<String> = HashSet::new();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert!(visible.is_empty());
}

#[test]
fn every_pack_declares_the_tools_it_is_named_for() {
    // Membership is compiled-in data, so a typo here is invisible until a
    // `use_skill` at runtime renders a pack that withheld nothing. Pin the
    // exact set per pack rather than a count.
    let expect: &[(&str, &[&str])] = &[
        (
            "workflows",
            &[
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
                "list_agent_definitions",
                "list_connectable_toolkits",
            ],
        ),
        (
            "web3",
            &[
                "wallet_status",
                "wallet_chain_status",
                "wallet_prepare_transfer",
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
            "mcp",
            &[
                "mcp_registry_status",
                "mcp_registry_search",
                "mcp_registry_get",
                "mcp_registry_installed_list",
                "mcp_registry_list_tools",
                "mcp_registry_connect",
                "mcp_registry_disconnect",
                "mcp_registry_tool_call",
                "mcp_registry_uninstall",
            ],
        ),
        (
            "composio",
            &[
                "composio",
                "composio_authorize",
                // `composio_connect` is the orchestrator's inline connect
                // card and stays unpacked (see the registry note).
                "composio_execute",
                "composio_list_connections",
                // `composio_list_toolkits` is the orchestrator's catalogue
                // lookup and stays unpacked (see the registry note): packed,
                // `closed_by_direct_handoff` denied it to the orchestrator and
                // the agent scraped the web for a list the app already had.
                "composio_list_tools",
            ],
        ),
        (
            "skills",
            &[
                "skill_search",
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
                "session_state",
                "credential_list",
                "oauth_connect_url",
                "oauth_list",
                "update_check",
                "update_apply",
            ],
        ),
        (
            "coding",
            &[
                // No `file_write`: it is the only create-capable tool on any
                // belt, and packing it denied the orchestrator every route to a
                // new file. No `file_read`: it is the tool every
                // `[tool_result_preview]` names. See the comments in
                // `registry.rs`.
                "grep",
                "glob",
                "list",
                "git_operations",
                "edit",
                "curl",
                "read_diff",
                "run_linter",
                "run_tests",
            ],
        ),
        (
            "storage",
            &[
                "storage_upload_file",
                "storage_download_file",
                "storage_list_files",
                "storage_get_link",
            ],
        ),
        (
            "scheduling",
            &[
                "cron_add",
                "cron_list",
                "cron_update",
                "cron_remove",
                "cron_run",
                "cron_runs",
            ],
        ),
        (
            "media",
            &[
                "media_generate_image",
                "media_generate_video",
                "media_list_models",
            ],
        ),
        ("goals", &["goal_get", "goal_set"]),
        ("docs", &["gitbooks_search", "gitbooks_get_page"]),
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
    // `workflow_builder` owns the workflows family: inspecting and authoring
    // flows IS its loop. Withholding its own belt would buy a `use_skill`
    // round trip per turn and hide nothing that is idle.
    let mut visible: HashSet<String> = ["propose_workflow".to_string(), "shell".to_string()]
        .into_iter()
        .collect();
    strip_packed_from_visible(&mut visible, "workflow_builder");
    assert!(
        visible.contains("propose_workflow"),
        "the workflows pack's owner lost its own tool"
    );
    assert!(!visible.contains(USE_SKILL), "owner gained pack tools");
}

#[test]
fn orchestrator_keeps_its_named_mcp_tools_advertised() {
    let mut visible: HashSet<String> = [
        "mcp_registry_status".to_string(),
        "mcp_registry_list_tools".to_string(),
        "mcp_registry_tool_call".to_string(),
    ]
    .into_iter()
    .collect();
    strip_packed_from_visible(&mut visible, "orchestrator");
    assert!(visible.contains("mcp_registry_status"));
    assert!(visible.contains("mcp_registry_list_tools"));
    assert!(visible.contains("mcp_registry_tool_call"));
}

#[test]
fn thread_renamed_orchestrator_keeps_its_mcp_tools_advertised() {
    let pack = registry::pack("mcp").expect("MCP pack");
    assert!(pack.is_owner("orchestrator_thread-mcp"));
    assert!(!pack.is_owner("orchestratorish_thread-mcp"));

    let mut visible: HashSet<String> = ["mcp_registry_tool_call".to_string()].into_iter().collect();
    strip_packed_from_visible(&mut visible, "orchestrator_thread-mcp");
    assert!(visible.contains("mcp_registry_tool_call"));
}

#[test]
fn a_packs_owner_still_loses_every_other_pack() {
    // Ownership is per pack, not a blanket exemption: `workflow_builder` owns
    // `workflows` and `composio`, and must still lose `web3`.
    let mut visible: HashSet<String> =
        ["propose_workflow".to_string(), "wallet_status".to_string()]
            .into_iter()
            .collect();
    strip_packed_from_visible(&mut visible, "workflow_builder");
    assert!(visible.contains("propose_workflow"));
    assert!(
        !visible.contains("wallet_status"),
        "non-owned pack survived"
    );
    assert!(visible.contains(USE_SKILL));
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

#[path = "toolpacks_tests_scoping_and_visibility_tests.rs"]
mod scoping_and_visibility_tests;

#[path = "toolpacks_tests_guides_tests.rs"]
mod guides_tests;
