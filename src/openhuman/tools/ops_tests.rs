use super::*;
use crate::openhuman::config::{BrowserConfig, Config, MemoryConfig};
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use tempfile::TempDir;

#[path = "../integrations/test_support.rs"]
mod integration_test_support;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
}

fn test_memory(tmp: &TempDir) -> Arc<dyn Memory> {
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap())
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
    cfg.integrations.apify.enabled = true;
    cfg.integrations.google_places.enabled = true;
    cfg.integrations.parallel.enabled = true;
    cfg.integrations.stock_prices.enabled = true;
    cfg.integrations.twilio.enabled = true;
    cfg
}

fn integration_tools_for_config(tmp: &TempDir, cfg: &Config) -> Vec<Box<dyn Tool>> {
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    all_tools(
        Arc::new(cfg.clone()),
        &security,
        mem,
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

#[test]
fn default_tools_has_three() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(security);
    assert_eq!(tools.len(), 3);
}

#[test]
fn all_tools_includes_spawn_subagent() {
    // Regression guard: the `spawn_subagent` tool must be present
    // in the default registry so parent agents can delegate to
    // sub-agents at runtime. If this test fails, the dispatch path
    // in `agent::harness::subagent_runner` becomes unreachable.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec![],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"spawn_subagent"),
        "spawn_subagent must be registered in the default tool list; got: {names:?}"
    );
}

#[test]
fn all_tools_includes_spawn_parallel_agents() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());
    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec![],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"spawn_parallel_agents"),
        "spawn_parallel_agents must be registered for orchestrated fan-out; got: {names:?}"
    );
}

#[test]
fn all_tools_always_registers_curl() {
    // Regression guard: `curl` is always registered (gated only by
    // the shared `http_request.allowed_domains` allowlist at call
    // time, like `http_request`). `Write` permission level keeps it
    // off agents that aren't allowed to modify the workspace.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"curl"),
        "curl must always be registered; got: {names:?}"
    );
}

#[test]
fn all_tools_registers_gitbooks_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.gitbooks.enabled = true;

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"gitbooks_search"),
        "gitbooks_search must register when gitbooks.enabled = true; got: {names:?}"
    );
    assert!(
        names.contains(&"gitbooks_get_page"),
        "gitbooks_get_page must register when gitbooks.enabled = true; got: {names:?}"
    );
}

#[test]
fn all_tools_registers_generic_mcp_bridge_tools_when_servers_exist() {
    let tmp = TempDir::new().unwrap();
    let mut cfg = test_config(&tmp);
    cfg.gitbooks.enabled = false;
    cfg.mcp_client
        .servers
        .push(crate::openhuman::config::McpServerConfig {
            name: "docs".into(),
            endpoint: "https://example.com/mcp".into(),
            command: String::new(),
            args: Vec::new(),
            env: std::collections::HashMap::new(),
            cwd: None,
            description: Some("Example docs MCP".into()),
            enabled: true,
            timeout_secs: 30,
            auth: crate::openhuman::config::McpAuthConfig::None,
        });

    let tools = integration_tools_for_config(&tmp, &cfg);
    let names = tool_names(&tools);
    assert_contains_all(
        &names,
        &["mcp_list_servers", "mcp_list_tools", "mcp_call_tool"],
    );
}

#[test]
fn all_tools_skips_gitbooks_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.gitbooks.enabled = false;

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        !names.contains(&"gitbooks_search"),
        "gitbooks_search must NOT register when gitbooks.enabled = false; got: {names:?}"
    );
    assert!(
        !names.contains(&"gitbooks_get_page"),
        "gitbooks_get_page must NOT register when gitbooks.enabled = false; got: {names:?}"
    );
}

#[test]
fn all_tools_includes_complete_onboarding() {
    // Regression guard: the `complete_onboarding` tool must be
    // present so the welcome agent can check setup status and
    // finalize onboarding.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"complete_onboarding"),
        "complete_onboarding must be registered in the default tool list; got: {names:?}"
    );
    assert!(
        names.contains(&"check_onboarding_status"),
        "check_onboarding_status must be registered in the default tool list; got: {names:?}"
    );
}

#[test]
fn all_tools_includes_current_time() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"current_time"),
        "current_time must be registered in the default tool list; got: {names:?}"
    );
}

#[test]
fn all_tools_default_registry_contains_expected_baseline_surface() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig {
        enabled: false,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);

    assert_contains_all(
        &names,
        &[
            "shell",
            "file_read",
            "file_write",
            "grep",
            "glob",
            "list",
            "edit",
            "apply_patch",
            "csv_export",
            "spawn_subagent",
            "spawn_parallel_agents",
            "todowrite",
            "plan_exit",
            "check_onboarding_status",
            "complete_onboarding",
            "current_time",
            "cron_add",
            "cron_list",
            "cron_remove",
            "cron_update",
            "cron_run",
            "cron_runs",
            "memory_store",
            "memory_recall",
            "memory_forget",
            "memory_tree",
            "whatsapp_data_list_chats",
            "whatsapp_data_list_messages",
            "whatsapp_data_search_messages",
            "schedule",
            "proxy_config",
            "update_check",
            "update_apply",
            "git_operations",
            "pushover",
            "gmail_unsubscribe",
            "http_request",
            "web_fetch",
            "curl",
            "gitbooks_search",
            "gitbooks_get_page",
            "web_search_tool",
            "node_exec",
            "npm_exec",
            "screenshot",
            "image_info",
        ],
    );
}

#[test]
fn all_tools_default_registry_has_no_duplicate_tool_names() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig {
        enabled: false,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    let unique: std::collections::HashSet<_> = names.iter().cloned().collect();
    assert_eq!(
        unique.len(),
        names.len(),
        "tool registry must not contain duplicate names: {names:?}"
    );
}

#[test]
fn all_tools_excludes_browser_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec!["example.com".into()],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"browser_open"));
    assert!(names.contains(&"schedule"));
    assert!(names.contains(&"pushover"));
    assert!(names.contains(&"proxy_config"));
}

#[test]
fn all_tools_includes_browser_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig {
        enabled: true,
        allowed_domains: vec!["example.com".into()],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"browser_open"));
    assert!(names.contains(&"pushover"));
    assert!(names.contains(&"proxy_config"));
}

#[test]
fn default_tools_names() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(security);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"shell"));
    assert!(names.contains(&"file_read"));
    assert!(names.contains(&"file_write"));
}

#[test]
fn default_tools_all_have_descriptions() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(security);
    for tool in &tools {
        assert!(
            !tool.description().is_empty(),
            "Tool {} has empty description",
            tool.name()
        );
    }
}

#[test]
fn default_tools_all_have_schemas() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(security);
    for tool in &tools {
        let schema = tool.parameters_schema();
        assert!(
            schema.is_object(),
            "Tool {} schema is not an object",
            tool.name()
        );
        assert!(
            schema["properties"].is_object(),
            "Tool {} schema has no properties",
            tool.name()
        );
    }
}

#[test]
fn tool_spec_generation() {
    let security = Arc::new(SecurityPolicy::default());
    let tools = default_tools(security);
    for tool in &tools {
        let spec = tool.spec();
        assert_eq!(spec.name, tool.name());
        assert_eq!(spec.description, tool.description());
        assert!(spec.parameters.is_object());
    }
}

#[test]
fn tool_result_serde() {
    let result = ToolResult::success("hello");
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ToolResult = serde_json::from_str(&json).unwrap();
    assert!(!parsed.is_error);
    assert_eq!(parsed.output(), "hello");
}

#[test]
fn tool_result_with_error_serde() {
    let result = ToolResult::error("boom");
    let json = serde_json::to_string(&result).unwrap();
    let parsed: ToolResult = serde_json::from_str(&json).unwrap();
    assert!(parsed.is_error);
    assert_eq!(parsed.output(), "boom");
}

#[test]
fn tool_spec_serde() {
    let spec = ToolSpec {
        name: "test".into(),
        description: "A test tool".into(),
        parameters: serde_json::json!({"type": "object"}),
    };
    let json = serde_json::to_string(&spec).unwrap();
    let parsed: ToolSpec = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.name, "test");
    assert_eq!(parsed.description, "A test tool");
}

#[test]
fn all_tools_includes_delegate_when_agents_configured() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let mut agents = HashMap::new();
    agents.insert(
        "researcher".to_string(),
        DelegateAgentConfig {
            model: "llama3".to_string(),
            system_prompt: None,
            temperature: None,
            max_depth: 3,
        },
    );

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &agents,
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(names.contains(&"delegate"));
}

#[test]
fn all_tools_excludes_delegate_when_no_agents() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"delegate"));
}

#[test]
fn all_tools_registers_node_exec_when_node_enabled() {
    // Default NodeConfig has `enabled = true`, so both `node_exec` and
    // `npm_exec` must appear in the registry. Regression guard for the
    // skills integration — if this fires, managed-node skills silently
    // lose both tools.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"node_exec"),
        "node_exec must be registered when node.enabled=true; got: {names:?}"
    );
    assert!(
        names.contains(&"npm_exec"),
        "npm_exec must be registered when node.enabled=true; got: {names:?}"
    );
}

#[test]
fn all_tools_excludes_node_exec_when_node_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.node.enabled = false;

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        !names.contains(&"node_exec"),
        "node_exec must NOT be registered when node.enabled=false; got: {names:?}"
    );
    assert!(
        !names.contains(&"npm_exec"),
        "npm_exec must NOT be registered when node.enabled=false; got: {names:?}"
    );
}

#[test]
fn all_tools_excludes_computer_control_when_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    // Default config has computer_control.enabled = false
    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        !names.contains(&"mouse"),
        "mouse tool should not be registered when computer_control.enabled=false"
    );
    assert!(
        !names.contains(&"keyboard"),
        "keyboard tool should not be registered when computer_control.enabled=false"
    );
}

#[test]
fn all_tools_includes_computer_control_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.computer_control.enabled = true;

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"mouse"),
        "mouse tool must be registered when computer_control.enabled=true; got: {names:?}"
    );
    assert!(
        names.contains(&"keyboard"),
        "keyboard tool must be registered when computer_control.enabled=true; got: {names:?}"
    );
}

#[test]
fn all_tools_registers_integration_families_when_enabled_and_signed_in() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.api_url = Some("https://backend.example.test".to_string());
    cfg.integrations.apify.enabled = true;
    cfg.integrations.google_places.enabled = true;
    cfg.integrations.parallel.enabled = true;
    cfg.integrations.stock_prices.enabled = true;
    cfg.integrations.twilio.enabled = true;
    cfg.composio.enabled = true;
    store_test_session_token(&cfg);

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);

    assert_contains_all(
        &names,
        &[
            "apify_run_actor",
            "apify_get_run_status",
            "apify_get_run_results",
            "google_places_search",
            "google_places_details",
            "parallel_search",
            "parallel_extract",
            "parallel_chat",
            "parallel_research",
            "parallel_enrich",
            "parallel_dataset",
            "stock_quote",
            "stock_exchange_rate",
            "stock_options",
            "stock_crypto_series",
            "stock_commodity",
            "twilio_call",
            "composio_list_toolkits",
            "composio_list_connections",
            "composio_authorize",
            "composio_list_tools",
            "composio_execute",
        ],
    );
}

#[test]
fn all_tools_registers_seltz_lsp_and_tool_stats_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.seltz.enabled = true;
    cfg.learning.enabled = true;
    cfg.learning.tool_tracking_enabled = true;

    let _env_guard = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var(
            crate::openhuman::tools::implementations::LSP_ENABLED_ENV,
            "1",
        );
    }

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    assert_contains_all(&names, &["seltz_search", "lsp", "tool_stats"]);

    unsafe {
        std::env::remove_var(crate::openhuman::tools::implementations::LSP_ENABLED_ENV);
    }
}

#[tokio::test]
async fn all_tools_executes_apify_family_against_fake_backend() {
    let backend = integration_test_support::spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);

    let run = find_tool(&tools, "apify_run_actor")
        .execute(serde_json::json!({
            "actor_id": "apify/linkedin-profile-scraper",
            "input": { "profile": "alice" },
            "sync": true,
            "timeout_secs": 45,
            "memory_mbytes": 512
        }))
        .await
        .expect("apify_run_actor execute");
    assert!(run.output().contains("apify/linkedin-profile-scraper"));
    assert!(run.output().contains("run-apify-linkedin-profile-scraper"));

    let status = find_tool(&tools, "apify_get_run_status")
        .execute(serde_json::json!({ "run_id": "run-apify-linkedin-profile-scraper" }))
        .await
        .expect("apify_get_run_status execute");
    assert!(status.output().contains("Status: SUCCEEDED"));
    assert!(status
        .output()
        .contains("dataset-run-apify-linkedin-profile-scraper"));

    let results = find_tool(&tools, "apify_get_run_results")
        .execute(serde_json::json!({
            "run_id": "run-apify-linkedin-profile-scraper",
            "limit": 2,
            "offset": 1
        }))
        .await
        .expect("apify_get_run_results execute");
    assert!(results.output().contains("Fetched 2 dataset item(s)."));
    assert!(results
        .output()
        .contains("https://example.com/run-apify-linkedin-profile-scraper/1"));

    let requests = backend.requests();
    assert_eq!(requests.len(), 3);
    assert_eq!(requests[0].path, "/agent-integrations/apify/run");
    assert_eq!(
        requests[0].body["actorId"],
        serde_json::json!("apify/linkedin-profile-scraper")
    );
    assert_eq!(requests[0].body["memoryMbytes"], serde_json::json!(512));
    assert_eq!(
        requests[2].path,
        "/agent-integrations/apify/runs/run-apify-linkedin-profile-scraper/results?limit=2&offset=1"
    );
}

#[tokio::test]
async fn all_tools_executes_google_places_family_against_fake_backend() {
    let backend = integration_test_support::spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);

    let search = find_tool(&tools, "google_places_search")
        .execute(serde_json::json!({
            "query": "coffee",
            "max_results": 2
        }))
        .await
        .expect("google_places_search execute");
    assert!(search.output().contains("Found 2 place(s) for: coffee"));
    assert!(search.output().contains("coffee Result 1"));

    let details = find_tool(&tools, "google_places_details")
        .execute(serde_json::json!({ "place_id": "place-1-coffee" }))
        .await
        .expect("google_places_details execute");
    assert!(details.output().contains("Details for place-1-coffee"));
    assert!(details.output().contains("OPERATIONAL"));

    let requests = backend.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].body["maxResults"], serde_json::json!(2));
    assert_eq!(
        requests[1].body["placeId"],
        serde_json::json!("place-1-coffee")
    );
}

#[tokio::test]
async fn all_tools_executes_parallel_and_web_search_family_against_fake_backend() {
    let backend = integration_test_support::spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);

    let web_search = find_tool(&tools, "web_search_tool")
        .execute(serde_json::json!({ "query": "rust testing" }))
        .await
        .expect("web_search_tool execute");
    assert!(web_search
        .output()
        .contains("Search results for: rust testing"));
    assert!(web_search.output().contains("Objective: rust testing"));

    let parallel_search = find_tool(&tools, "parallel_search")
        .execute(serde_json::json!({
            "objective": "tool wiring",
            "search_queries": ["tool wiring", "mock backend"],
            "num_results": 3,
            "max_characters_per_excerpt": 200
        }))
        .await
        .expect("parallel_search execute");
    assert!(parallel_search
        .output()
        .contains("Search results (2 found):"));
    assert!(parallel_search.output().contains("Result for tool wiring"));
    assert!(parallel_search.output().contains("Objective: tool wiring"));

    let extract = find_tool(&tools, "parallel_extract")
        .execute(serde_json::json!({
            "urls": ["https://example.com/a"],
            "objective": "capture the summary",
            "full_content": true
        }))
        .await
        .expect("parallel_extract execute");
    assert!(extract.output().contains("Extracted https://example.com/a"));
    assert!(extract
        .output()
        .contains("Full content for https://example.com/a"));

    let chat = find_tool(&tools, "parallel_chat")
        .execute(serde_json::json!({
            "model": "base",
            "messages": [{ "role": "user", "content": "what changed?" }]
        }))
        .await
        .expect("parallel_chat execute");
    assert!(chat.output().contains("Model base answered: what changed?"));
    assert!(chat.output().contains("\"sources\""));

    let research = find_tool(&tools, "parallel_research")
        .execute(serde_json::json!({
            "input": { "company": "Tiny Humans" },
            "processor": "core",
            "timeout_seconds": 30
        }))
        .await
        .expect("parallel_research execute");
    assert!(research.output().contains("Run: research-core"));
    assert!(research.output().contains("\"company\": \"Tiny Humans\""));

    let enrich = find_tool(&tools, "parallel_enrich")
        .execute(serde_json::json!({
            "input": "Tiny Humans",
            "processor": "lite",
            "output_schema": { "type": "object" }
        }))
        .await
        .expect("parallel_enrich execute");
    assert!(enrich.output().contains("Enriched entity"));
    assert!(enrich.output().contains("\"inputEcho\": \"Tiny Humans\""));

    let dataset = find_tool(&tools, "parallel_dataset")
        .execute(serde_json::json!({
            "objective": "Find AI startups",
            "entity_type": "company",
            "match_conditions": [{ "name": "AI-focused" }],
            "generator": "base",
            "match_limit": 25
        }))
        .await
        .expect("parallel_dataset execute");
    assert!(dataset.output().contains("findall_id: dataset-company"));
    assert!(dataset.output().contains("match_limit: 25"));

    let requests = backend.requests();
    let paths: Vec<&str> = requests.iter().map(|req| req.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/agent-integrations/parallel/search",
            "/agent-integrations/parallel/search",
            "/agent-integrations/parallel/extract",
            "/agent-integrations/parallel/chat",
            "/agent-integrations/parallel/research",
            "/agent-integrations/parallel/enrich",
            "/agent-integrations/parallel/dataset",
        ]
    );
    assert_eq!(
        requests[1].body["excerpts"]["numResults"],
        serde_json::json!(3)
    );
    assert_eq!(requests[2].body["fullContent"], serde_json::json!(true));
    assert_eq!(requests[6].body["matchLimit"], serde_json::json!(25));
}

#[tokio::test]
async fn all_tools_executes_stock_and_twilio_family_against_fake_backend() {
    let backend = integration_test_support::spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);

    let quote = find_tool(&tools, "stock_quote")
        .execute(serde_json::json!({ "symbol": "AAPL" }))
        .await
        .expect("stock_quote execute");
    assert!(quote.output().contains("AAPL"));
    assert!(quote.output().contains("latest trading day 2026-05-16"));

    let exchange = find_tool(&tools, "stock_exchange_rate")
        .execute(serde_json::json!({
            "from_currency": "BTC",
            "to_currency": "USD"
        }))
        .await
        .expect("stock_exchange_rate execute");
    assert!(exchange.output().contains("BTC/USD = 42.5"));

    let options = find_tool(&tools, "stock_options")
        .execute(serde_json::json!({
            "symbol": "AAPL",
            "require_greeks": true
        }))
        .await
        .expect("stock_options execute");
    assert!(options.output().contains("AAPL options chain"));
    assert!(options.output().contains("call 2026-06-19 @ 250"));

    let crypto = find_tool(&tools, "stock_crypto_series")
        .execute(serde_json::json!({
            "symbol": "BTC",
            "market": "USD",
            "limit": 2
        }))
        .await
        .expect("stock_crypto_series execute");
    assert!(crypto.output().contains("BTC/USD"));
    assert!(crypto.output().contains("2026-05-16"));

    let commodity = find_tool(&tools, "stock_commodity")
        .execute(serde_json::json!({
            "commodity": "WTI",
            "interval": "weekly",
            "limit": 2
        }))
        .await
        .expect("stock_commodity execute");
    assert!(commodity.output().contains("WTI (weekly)"));
    assert!(commodity.output().contains("2026-05-16  80.1000"));

    let twilio = find_tool(&tools, "twilio_call")
        .execute(serde_json::json!({
            "to": "+14155551234",
            "message": "Hello from tests"
        }))
        .await
        .expect("twilio_call execute");
    assert!(twilio.output().contains("Call SID: CA1234"));
    assert!(twilio.output().contains("Status: queued"));

    let requests = backend.requests();
    let paths: Vec<&str> = requests.iter().map(|req| req.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/agent-integrations/financial-apis/quote",
            "/agent-integrations/financial-apis/exchange-rate",
            "/agent-integrations/financial-apis/options",
            "/agent-integrations/financial-apis/crypto-series",
            "/agent-integrations/financial-apis/commodity",
            "/agent-integrations/twilio/call",
        ]
    );
    assert_eq!(requests[2].body["requireGreeks"], serde_json::json!(true));
    assert_eq!(requests[5].body["to"], serde_json::json!("+14155551234"));
}
