use super::*;
use crate::openhuman::config::{BrowserConfig, Config, MemoryConfig};
use crate::openhuman::security::credentials::{
    AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME,
};
use crate::openhuman::security::AuditLogger;
use crate::openhuman::skills::types::ToolContent;
use tempfile::TempDir;

#[path = "../integrations/test_support.rs"]
mod integration_test_support;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn tool_names(tools: &[Box<dyn Tool>]) -> Vec<String> {
    tools.iter().map(|t| t.name().to_string()).collect()
}

fn assert_contains_all(names: &[String], expected: &[&str]) {
    for name in expected {
        assert!(
            names.iter().any(|n| n == name),
            "expected tool `{name}` to be registered; got: {names:?}"
        );
    }
}

fn only_json_content(result: &ToolResult) -> &serde_json::Value {
    match result.content.as_slice() {
        [ToolContent::Json { data }] => data,
        other => panic!("expected a single JSON content block, got {other:?}"),
    }
}

fn store_test_session_token(config: &Config) {
    AuthService::from_config(config)
        .store_provider_token(
            APP_SESSION_PROVIDER,
            DEFAULT_AUTH_PROFILE_NAME,
            "test-token",
            std::collections::HashMap::new(),
            true,
        )
        .expect("store test session token");
}

fn integration_test_config(tmp: &TempDir, backend_url: &str) -> Config {
    let mut cfg = test_config(tmp);
    cfg.api_url = Some(backend_url.to_string());
    cfg.integrations.google_places.enabled = true;
    cfg.integrations.parallel.enabled = true;
    cfg.integrations.tinyfish.enabled = true;
    cfg.integrations.stock_prices.enabled = true;
    cfg.integrations.twilio.enabled = true;
    // Parallel tools (search/extract/chat/research/enrich/dataset) are
    // registered by the unified search-engine selector, so flip the
    // engine to `parallel` in test setup.
    cfg.search.engine = crate::openhuman::config::SEARCH_ENGINE_PARALLEL.into();
    cfg.search.parallel.api_key = Some("test-parallel-key".into());
    cfg
}

fn integration_tools_for_config(tmp: &TempDir, cfg: &Config) -> Vec<Box<dyn Tool>> {
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        cfg,
    )
}

fn find_tool<'a>(tools: &'a [Box<dyn Tool>], name: &str) -> &'a dyn Tool {
    tools
        .iter()
        .find(|tool| tool.name() == name)
        .map(|tool| tool.as_ref())
        .unwrap_or_else(|| panic!("tool `{name}` not registered"))
}

// ── Agent-tool expansion: shared e2e harness ────────────────────────────────
//
// Both themes (Task & workflow productivity; Knowledge & memory) exercise the
// full `all_tools` registry: that every tool registers, that the overextending
// siblings are stripped by the user-filter when not opted in (and restored
// when opted in), and a couple of real executions through the boxed `dyn Tool`
// surface.

/// Build the full tool registry with a disabled browser and a tmp-scoped
/// workspace — enough to exercise the expansion tools end-to-end.
fn expansion_tools_for(tmp: &TempDir) -> Vec<Box<dyn Tool>> {
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec![],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(tmp);
    all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    )
}

// ── Theme: Task & workflow productivity ─────────────────────────────────────

const PRODUCTIVITY_TOOLS: &[&str] = &[
    // NOTE: the old `agent_workflow_*` tools were removed when the
    // `agent_workflows` domain was dissolved into `workflows`; workflow
    // discovery/run tools now live under the Knowledge theme
    // (`list_workflows`, `run_workflow`, …).
    "artifact_list",
    "artifact_get",
    "artifact_delete",
    "todo_list",
    "todo_add",
    "todo_edit",
    "todo_update_status",
    "todo_decide_plan",
    "todo_remove",
    "todo_replace",
    "todo_clear",
    "task_source_list",
    "task_source_get",
    "task_source_fetch",
    "task_source_list_tasks",
    "task_source_preview_filter",
    "task_source_status",
    "task_source_add",
    "task_source_update",
    "task_source_remove",
];

const PRODUCTIVITY_DEFAULT_OFF: &[&str] = &[
    "artifact_delete",
    "todo_remove",
    "todo_replace",
    "todo_clear",
    "task_source_add",
    "task_source_update",
    "task_source_remove",
];

const PRODUCTIVITY_ALWAYS_ON: &[&str] = &[
    "artifact_list",
    "artifact_get",
    "todo_list",
    "todo_add",
    "task_source_fetch",
    "task_source_status",
];

// ── Theme: Knowledge & memory ───────────────────────────────────────────────

const KNOWLEDGE_TOOLS: &[&str] = &[
    "list_workflows",
    "describe_workflow",
    "read_workflow_resource",
    "list_workflow_runs",
    "read_workflow_run_log",
    "create_skill",
    "install_workflow_from_url",
    "uninstall_workflow",
    "learning_list_facets",
    "learning_get_facet",
    "learning_cache_stats",
    "learning_update_facet",
    "learning_pin_facet",
    "learning_unpin_facet",
    "learning_forget_facet",
    "learning_rebuild_cache",
    "learning_reset_cache",
    "learning_save_profile",
    "learning_enrich_profile",
];

fn knowledge_default_off() -> Vec<&'static str> {
    let mut tools = vec![
        "learning_update_facet",
        "learning_pin_facet",
        "learning_unpin_facet",
        "learning_forget_facet",
        "learning_rebuild_cache",
        "learning_reset_cache",
        "learning_save_profile",
        "learning_enrich_profile",
    ];
    // These tools exist only when their feature gates are on. All of
    // create_skill / install_workflow_from_url / uninstall_workflow are
    // registered under `#[cfg(feature = "skills")]` in ops.rs — none of
    // them are behind `flows`.
    if cfg!(feature = "skills") {
        tools.push("create_skill");
        tools.push("install_workflow_from_url");
        tools.push("uninstall_workflow");
    }
    tools
}

fn knowledge_always_on() -> Vec<&'static str> {
    let mut tools = vec!["learning_list_facets", "learning_cache_stats"];
    // These tools exist only when the skills feature is on (`WorkflowListTool`
    // / `WorkflowRecentRunsTool` — both `#[cfg(feature = "skills")]`).
    if cfg!(feature = "skills") {
        tools.extend(&["list_workflows", "list_workflow_runs"]);
    }
    tools
}

// ── Theme: System & self-management (observability + service) ───────────────

const SYSTEM_TOOLS: &[&str] = &[
    "doctor_health",
    "doctor_models",
    "health_snapshot",
    "health_system_info",
    "cost_get_dashboard",
    "cost_get_daily_history",
    "cost_get_summary",
    "dashboard_model_health",
    "security_policy_info",
    "service_status",
    "daemon_host_prefs_get",
    "service_start",
    "service_stop",
    "service_restart",
    "service_shutdown",
    "service_install",
    "service_uninstall",
    "daemon_host_prefs_set",
    "config_snapshot",
    "config_get_client_config",
    "config_get_autonomy",
    "config_get_search",
    "config_get_runtime_flags",
    "config_resolve_api_url",
    "config_get_data_paths",
];

const SYSTEM_DEFAULT_OFF: &[&str] = &[
    "service_start",
    "service_stop",
    "service_restart",
    "service_shutdown",
    "service_install",
    "service_uninstall",
    "daemon_host_prefs_set",
];

const SYSTEM_ALWAYS_ON: &[&str] = &[
    "doctor_health",
    "health_snapshot",
    "cost_get_summary",
    "dashboard_model_health",
    "security_policy_info",
    "service_status",
    "daemon_host_prefs_get",
    "config_snapshot",
    "config_get_autonomy",
];

// ── Theme: Account & session ────────────────────────────────────────────────
//
// The `billing_*`, `team_*` and `referral_*` agent-tool families were removed:
// money movement and team administration are dashboard surfaces, and their
// controllers stay registered for the UI. What remains here is read-only
// account *state* — who is signed in, what is connected — which moved to
// `settings_agent` when `account_admin_agent` went with those families.

const ACCOUNT_TOOLS: &[&str] = &[
    "credential_list",
    "session_state",
    "session_get_user",
    "oauth_connect_url",
    "oauth_list",
];

// ── Theme: MCP registry and workspace ───────────────────────────────────────

const DESKTOP_TOOLS: &[&str] = &[
    // The `mcp_registry_*` desktop surface is compiled out with the `mcp`
    // feature, so these expectations are gated per-element rather than gating
    // the three tests below away wholesale — the non-MCP desktop tools must
    // keep their coverage in both builds.
    #[cfg(feature = "mcp")]
    "mcp_registry_search",
    #[cfg(feature = "mcp")]
    "mcp_registry_get",
    #[cfg(feature = "mcp")]
    "mcp_registry_installed_list",
    #[cfg(feature = "mcp")]
    "mcp_registry_status",
    #[cfg(feature = "mcp")]
    "mcp_registry_connect",
    #[cfg(feature = "mcp")]
    "mcp_registry_disconnect",
    #[cfg(feature = "mcp")]
    "mcp_registry_tool_call",
    #[cfg(feature = "mcp")]
    "mcp_registry_config_assist",
    #[cfg(feature = "mcp")]
    "mcp_registry_install",
    #[cfg(feature = "mcp")]
    "mcp_registry_uninstall",
    "workspace_read_persona",
    "workspace_update_persona",
    "workspace_reset_persona",
    "workspace_init",
];

const DESKTOP_DEFAULT_OFF: &[&str] = &[
    #[cfg(feature = "mcp")]
    "mcp_registry_install",
    #[cfg(feature = "mcp")]
    "mcp_registry_uninstall",
    "workspace_update_persona",
    "workspace_reset_persona",
    "workspace_init",
];

const DESKTOP_ALWAYS_ON: &[&str] = &[
    #[cfg(feature = "mcp")]
    "mcp_registry_search",
    #[cfg(feature = "mcp")]
    "mcp_registry_tool_call",
    #[cfg(feature = "mcp")]
    "mcp_registry_connect",
    "workspace_read_persona",
];

/// One real tool name per family that owns tools.
const REPRESENTATIVE: &[(&str, crate::core::all::DomainGroup)] = {
    use crate::core::all::DomainGroup as G;
    &[
        ("delegate", G::Agent),
        ("memory_search", G::Memory),
        ("todo_add", G::Threads),
        ("mcp_list_servers", G::Mcp),
        ("wallet_get_address", G::Web3),
        ("media_generate_image", G::Media),
        ("audio_generate_podcast", G::Voice),
        ("create_workflow", G::Flows),
        ("run_workflow", G::Skills),
        ("cron_add", G::Automation),
        ("composio_execute", G::Integrations),
        ("dashboard_model_health", G::Desktop),
        ("node_exec", G::Runtimes),
        ("tinyjuice_retrieve", G::Inference),
        ("shell", G::Platform),
    ]
};

/// Families with no agent tools of their own.
const TOOL_LESS: &[crate::core::all::DomainGroup] = {
    use crate::core::all::DomainGroup as G;
    // `Modules` is the loader, not a capability: a loaded module's own surface
    // is reached through whichever domain calls it (documents go through the
    // document tools), so the family itself owns no agent tool.
    // `Channels` joined this list when the three `whatsapp_data_*` tools went —
    // the channel runtime,
    // its controllers and its inbound dispatch are all still there.
    &[
        G::Config,
        G::Security,
        G::Medulla,
        G::Modules,
        G::Channels,
        G::Hosted,
    ]
};

// ---- tool_capability() drift guard (M5.3) ----------------------------------

/// Driver-backed memory tools and the capability each requires.
const MEMORY_TOOL_CAPABILITIES: &[(&str, tinymemory_api::capabilities::Capability)] = {
    use tinymemory_api::capabilities::Capability as C;
    &[
        ("memory_store", C::Core),
        ("memory_forget", C::Core),
        ("remember_preference", C::Core),
        ("save_preference", C::Core),
        ("memory_recall", C::Recall),
        ("memory_vector_search", C::Recall),
        ("memory_chunk_context", C::Recall),
        ("memory_hybrid_search", C::Recall),
        ("memory_store_raw_chunks", C::Recall),
        ("memory_tree", C::Tree),
        ("memory_flavour", C::Tree),
        ("memory_store_raw_search", C::Entities),
        ("memory_doctor", C::Maintenance),
        ("tool_stats", C::ToolMemory),
        ("goals", C::Goals),
    ]
};

/// Memory-family tools that are deliberately NOT driver-backed. Each entry is
/// an argument, not an omission — see `tool_capability`.
const MEMORY_TOOLS_NOT_DRIVER_BACKED: &[&str] = &["update_memory_md", "memory_store_kinds"];

// ---- both-ways: the capability post-filter (M5.3) --------------------------
//
// The ABSENT half is the one that proves the filter removes anything.

/// A distinct workspace per test: the memory binding cache is keyed by
/// workspace dir, so sharing one path between an ON and an OFF test would make
/// one of them silently assert the other's driver (the `caps_ws` convention
/// from `core::all_tests`).
fn caps_tools_ws(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("oh-m53-tools-{name}"))
}

/// `[subsystems.memory] driver = "null"` — `NullMemoryProvider` advertises
/// exactly `Capability::MANDATORY` = {core, recall, portability}, so every
/// optional family is OFF at once. An operator who wrote `driver = "null"` is
/// honoured rather than falling back (`memory::binding`).
fn null_driver_memory_cfg() -> crate::openhuman::config::schema::MemorySubsystemConfig {
    crate::openhuman::config::schema::MemorySubsystemConfig {
        driver: "null".into(),
        ..Default::default()
    }
}

/// The optional-family tools that must vanish under a driver advertising
/// nothing optional.
///
const OPTIONAL_FAMILY_MEMORY_TOOLS: &[&str] = &[
    "memory_tree",
    "memory_flavour",
    "memory_store_raw_search",
    "memory_doctor",
    "goals",
];

/// Memory-family tools that remain available when a null driver deliberately
/// disables every driver-backed capability.
const ALWAYS_PRESENT_MEMORY_TOOLS: &[&str] = &["update_memory_md", "memory_store_kinds"];

#[path = "ops_tests_part_01_tests.rs"]
mod part_01_tests;
#[path = "ops_tests_part_02_tests.rs"]
mod part_02_tests;
#[path = "ops_tests_part_03_tests.rs"]
mod part_03_tests;
#[path = "ops_tests_part_04_tests.rs"]
mod part_04_tests;
