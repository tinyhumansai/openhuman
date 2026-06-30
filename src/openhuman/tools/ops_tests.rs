use super::*;
use crate::openhuman::config::{BrowserConfig, Config, MemoryConfig};
use crate::openhuman::credentials::{AuthService, APP_SESSION_PROVIDER, DEFAULT_AUTH_PROFILE_NAME};
use crate::openhuman::security::AuditLogger;
use crate::openhuman::workflows::types::ToolContent;
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

fn test_memory(tmp: &TempDir) -> Arc<dyn Memory> {
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap())
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
    cfg.integrations.apify.enabled = true;
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
    let mem = test_memory(tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

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
        AuditLogger::disabled(),
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
fn all_tools_includes_spawn_async_subagent() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());
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
        AuditLogger::disabled(),
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"spawn_async_subagent"),
        "spawn_async_subagent must be registered for fire-and-forget background orchestration; got: {names:?}"
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());
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
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.gitbooks.enabled = true;

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
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
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.gitbooks.enabled = false;

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
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
fn all_tools_includes_current_time() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
    let mem: Arc<dyn Memory> =
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
        AuditLogger::disabled(),
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
            "spawn_async_subagent",
            "spawn_parallel_agents",
            "wait",
            "wait_loop",
            "todo",
            "plan_exit",
            "current_time",
            "resolve_time",
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
            "monitor",
            "monitor_list",
            "monitor_stop",
            "monitor_read",
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
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

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
        AuditLogger::disabled(),
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
fn browser_allowed_domains_shares_fetch_list_minus_wildcard() {
    // Unified web-access firewall: the browser tool derives its host allowlist
    // from `http_request.allowed_domains`, but the `"*"` allow-all wildcard is
    // stripped so a fetch-side "Allow all" never silently opens the browser.

    // Explicit hosts pass straight through (shared with fetch).
    assert_eq!(
        browser_allowed_domains(&["reuters.com".into(), "github.com".into()]),
        vec!["reuters.com".to_string(), "github.com".to_string()],
    );

    // `"*"` (fetch allow-all, and the http_request default) yields an EMPTY
    // browser list — browser stays closed unless OPENHUMAN_BROWSER_ALLOW_ALL.
    assert!(browser_allowed_domains(&["*".into()]).is_empty());

    // Mixed: wildcard dropped, explicit hosts kept.
    assert_eq!(
        browser_allowed_domains(&["*".into(), "intranet.corp".into()]),
        vec!["intranet.corp".to_string()],
    );

    // Block-all (empty fetch list) -> empty browser list.
    assert!(browser_allowed_domains(&[]).is_empty());
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

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
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

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
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.node.enabled = false;

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    // Default config has computer_control.enabled = false
    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
        Arc::from(crate::openhuman::memory_store::create_memory(&mem_cfg, tmp.path()).unwrap());

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.computer_control.enabled = true;

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
    cfg.integrations.tinyfish.enabled = true;
    cfg.integrations.stock_prices.enabled = true;
    cfg.integrations.twilio.enabled = true;
    cfg.composio.enabled = true;
    // Parallel tools now register through the unified search-engine selector.
    cfg.search.engine = crate::openhuman::config::SEARCH_ENGINE_PARALLEL.into();
    cfg.search.parallel.api_key = Some("test-parallel-key".into());
    store_test_session_token(&cfg);

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
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
            "tinyfish_search",
            "tinyfish_fetch",
            "tinyfish_agent_run",
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
fn all_tools_registers_brave_engine_lsp_and_tool_stats_when_enabled() {
    // The legacy seltz/searxng tools are no longer registered — the
    // unified `search.engine` selector replaces them. This test now
    // verifies that picking `brave` layers in its full tool surface
    // alongside lsp + tool_stats.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.search.engine = crate::openhuman::config::SEARCH_ENGINE_BRAVE.into();
    cfg.search.brave.api_key = Some("test-brave-key".into());
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
        AuditLogger::disabled(),
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
            "web_search_tool",
            "brave_news_search",
            "brave_image_search",
            "brave_video_search",
            "lsp",
            "tool_stats",
        ],
    );

    unsafe {
        std::env::remove_var(crate::openhuman::tools::implementations::LSP_ENABLED_ENV);
    }
}

#[test]
fn all_tools_registers_querit_engine_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.search.engine = crate::openhuman::config::SEARCH_ENGINE_QUERIT.into();
    cfg.search.querit.api_key = Some("test-querit-key".into());

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    assert_contains_all(&names, &["web_search_tool", "querit_search"]);
}

#[test]
fn all_tools_omits_search_surface_when_search_is_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let mem = test_memory(&tmp);
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.api_url = Some("https://backend.example.test".to_string());
    cfg.search.engine = crate::openhuman::config::SEARCH_ENGINE_DISABLED.into();
    cfg.search.brave.api_key = Some("test-brave-key".into());
    cfg.search.querit.api_key = Some("test-querit-key".into());
    cfg.integrations.tinyfish.enabled = true;
    store_test_session_token(&cfg);

    let tools = all_tools(
        Arc::new(cfg.clone()),
        &security,
        AuditLogger::disabled(),
        mem,
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);

    for search_tool in [
        "web_search_tool",
        "brave_news_search",
        "brave_image_search",
        "brave_video_search",
        "querit_search",
        "tinyfish_search",
        "tinyfish_fetch",
        "tinyfish_agent_run",
    ] {
        assert!(
            !names.iter().any(|name| name == search_tool),
            "did not expect search tool `{search_tool}` when search is disabled; got: {names:?}"
        );
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
    let run_display = run.output_for_llm(true);
    assert!(run_display.contains("apify/linkedin-profile-scraper"));
    assert!(!run_display.contains("run-apify-linkedin-profile-scraper"));
    let run_payload = only_json_content(&run);
    assert_eq!(
        run_payload["run_id"],
        serde_json::json!("run-apify-linkedin-profile-scraper")
    );

    let status = find_tool(&tools, "apify_get_run_status")
        .execute(serde_json::json!({ "run_id": "run-apify-linkedin-profile-scraper" }))
        .await
        .expect("apify_get_run_status execute");
    let status_display = status.output_for_llm(true);
    let status_prose = status_display
        .split("[apify_run_ref]")
        .next()
        .expect("status prose before ref");
    assert!(status_display.contains("Status: SUCCEEDED"));
    assert!(!status_prose.contains("run-apify-linkedin-profile-scraper"));
    assert!(!status_prose.contains("dataset-run-apify-linkedin-profile-scraper"));
    assert!(status_display.contains("[apify_run_ref]"));
    let status_payload = only_json_content(&status);
    assert_eq!(
        status_payload["run_id"],
        serde_json::json!("run-apify-linkedin-profile-scraper")
    );
    assert_eq!(
        status_payload["dataset_id"],
        serde_json::json!("dataset-run-apify-linkedin-profile-scraper")
    );

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
    let research_display = research.output_for_llm(true);
    assert!(research_display.contains("Status: completed"));
    assert!(research_display.contains("\"company\": \"Tiny Humans\""));
    assert!(!research_display.contains("research-core"));
    let research_payload = only_json_content(&research);
    assert!(research_payload.get("run_id").is_none());

    let enrich = find_tool(&tools, "parallel_enrich")
        .execute(serde_json::json!({
            "input": "Tiny Humans",
            "processor": "lite",
            "output_schema": { "type": "object" }
        }))
        .await
        .expect("parallel_enrich execute");
    let enrich_display = enrich.output_for_llm(true);
    assert!(enrich_display.contains("Enriched entity"));
    assert!(enrich_display.contains("\"inputEcho\": \"Tiny Humans\""));
    assert!(!enrich_display.contains("enrich-1"));
    let enrich_payload = only_json_content(&enrich);
    assert!(enrich_payload.get("run_id").is_none());

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
async fn all_tools_executes_tinyfish_family_against_fake_backend() {
    let backend = integration_test_support::spawn_fake_integration_backend().await;
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, &backend.base_url);
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);

    let search = find_tool(&tools, "tinyfish_search")
        .execute(serde_json::json!({
            "query": "web automation",
            "location": "US",
            "language": "en",
            "page": 2,
            "include_thumbnail": true
        }))
        .await
        .expect("tinyfish_search execute");
    assert!(search
        .output()
        .contains("TinyFish returned 1 search result(s)"));
    assert!(search
        .output()
        .contains("TinyFish result for web automation"));

    let fetch = find_tool(&tools, "tinyfish_fetch")
        .execute(serde_json::json!({
            "urls": ["https://example.com/a"],
            "format": "markdown",
            "links": true,
            "image_links": true
        }))
        .await
        .expect("tinyfish_fetch execute");
    assert!(fetch.output().contains("TinyFish fetched 1 page(s)"));
    assert!(fetch
        .output()
        .contains("TinyFish content for https://example.com/a"));

    let run = find_tool(&tools, "tinyfish_agent_run")
        .execute(serde_json::json!({
            "url": "https://example.com/shop",
            "goal": "Extract product names. Return JSON.",
            "browser_profile": "stealth",
            "proxy_country_code": "US",
            "output_schema": { "type": "object" }
        }))
        .await
        .expect("tinyfish_agent_run execute");
    assert!(run.output().contains("TinyFish automation finished."));
    assert!(!run.output().contains("run_tinyfish_fake"));
    assert!(run.output().contains("\"ok\":true"));

    let requests = backend.requests();
    let paths: Vec<&str> = requests.iter().map(|req| req.path.as_str()).collect();
    assert_eq!(
        paths,
        vec![
            "/agent-integrations/tinyfish/search",
            "/agent-integrations/tinyfish/fetch",
            "/agent-integrations/tinyfish/agent/run",
        ]
    );
    assert_eq!(requests[0].body["location"], serde_json::json!("US"));
    assert_eq!(requests[1].body["links"], serde_json::json!(true));
    assert_eq!(
        requests[2].body["proxy_config"]["country_code"],
        serde_json::json!("US")
    );
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

/// Every acting tool gates on `can_act()` and returns its own read-only refusal
/// string. Each of those must carry [`POLICY_BLOCKED_MARKER`] so the agent
/// harness recognizes the block as a hard reject and halts on a verbatim repeat
/// (see `agent::harness::tool_loop::hard_reject_kind`). This pins every tool's
/// literal to the marker const — drift between them fails here rather than
/// silently letting the agent grind on a doomed call. Args are the minimum
/// needed to reach the `can_act()` check in each tool.
#[tokio::test]
async fn readonly_acting_tools_carry_policy_blocked_marker() {
    use crate::openhuman::security::{AutonomyLevel, POLICY_BLOCKED_MARKER};

    let tmp = TempDir::new().unwrap();
    let sec = Arc::new(SecurityPolicy {
        autonomy: AutonomyLevel::ReadOnly,
        workspace_dir: tmp.path().to_path_buf(),
        action_dir: tmp.path().to_path_buf(),
        ..SecurityPolicy::default()
    });

    let cases: Vec<(Box<dyn Tool>, serde_json::Value)> = vec![
        (
            Box::new(ApplyPatchTool::new(sec.clone())),
            serde_json::json!({ "edits": [{ "path": "a.txt", "old_string": "x", "new_string": "y" }] }),
        ),
        (
            Box::new(CsvExportTool::new(sec.clone())),
            serde_json::json!({ "data": "col1\nval1", "filename": "x.csv" }),
        ),
        (
            Box::new(KeyboardTool::new(sec.clone())),
            serde_json::json!({}),
        ),
        (Box::new(MouseTool::new(sec.clone())), serde_json::json!({})),
        (
            Box::new(BrowserOpenTool::new(sec.clone(), vec![])),
            serde_json::json!({ "url": "https://example.com" }),
        ),
        (
            Box::new(HttpRequestTool::new(sec.clone(), vec![], 0, 0)),
            serde_json::json!({ "url": "https://example.com" }),
        ),
    ];

    for (tool, args) in cases {
        let name = tool.name().to_string();
        let out = tool.execute(args).await.unwrap();
        assert!(out.is_error, "{name} should error under read-only autonomy");
        assert!(
            out.output().contains(POLICY_BLOCKED_MARKER),
            "{name} read-only block must carry {POLICY_BLOCKED_MARKER}, got: {}",
            out.output()
        );
    }
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
    let mem = test_memory(tmp);
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
        mem,
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

#[test]
fn productivity_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, PRODUCTIVITY_TOOLS);
}

#[test]
fn productivity_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in PRODUCTIVITY_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in PRODUCTIVITY_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn productivity_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(
        &mut tools,
        &[
            "todo_destructive".to_string(),
            "task_source_manage".to_string(),
            "artifact_delete".to_string(),
        ],
    );
    let names = tool_names(&tools);
    for on in PRODUCTIVITY_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}

#[tokio::test]
async fn todo_tools_add_then_list_through_registry() {
    // Drive the boxed `dyn Tool` surface exactly as the agent loop would: add
    // a card, then list it back. Thread-scoped (file-backed under the tmp
    // workspace) so the board is isolated from the process-global scratch
    // store and from parallel tests.
    let tmp = TempDir::new().unwrap();
    let tools = expansion_tools_for(&tmp);

    let add = find_tool(&tools, "todo_add");
    let added = add
        .execute(serde_json::json!({ "thread_id": "e2e-thread", "content": "registry e2e task" }))
        .await
        .expect("todo_add execute");
    assert!(added.output_for_llm(false).contains("registry e2e task"));

    let list = find_tool(&tools, "todo_list");
    let listed = list
        .execute(serde_json::json!({ "thread_id": "e2e-thread" }))
        .await
        .expect("todo_list execute");
    assert!(listed.output_for_llm(false).contains("registry e2e task"));
}

#[tokio::test]
async fn artifact_list_through_registry_returns_envelope() {
    let tmp = TempDir::new().unwrap();
    let tools = expansion_tools_for(&tmp);
    let out = find_tool(&tools, "artifact_list")
        .execute(serde_json::json!({ "limit": 10 }))
        .await
        .expect("artifact_list execute");
    let body = out.output_for_llm(false);
    assert!(body.contains("artifacts"), "envelope missing: {body}");
    assert!(body.contains("total"), "envelope missing total: {body}");
}

// ── Theme: Knowledge & memory ───────────────────────────────────────────────

const KNOWLEDGE_TOOLS: &[&str] = &[
    "people_list",
    "people_resolve",
    "people_score",
    "people_get",
    "people_add_alias",
    "people_record_interaction",
    "people_refresh_address_book",
    "list_workflows",
    "describe_workflow",
    "read_workflow_resource",
    "list_workflow_runs",
    "read_workflow_run_log",
    "create_workflow",
    "install_workflow_from_url",
    "uninstall_workflow",
    "thread_list",
    "thread_read",
    "thread_create",
    "thread_update_title",
    "thread_update_labels",
    "thread_message_list",
    "thread_message_append",
    "thread_message_update",
    "thread_title_generate",
    "thread_turn_state_get",
    "thread_turn_state_list",
    "thread_turn_state_clear",
    "thread_task_board_read",
    "thread_task_board_write",
    "thread_delete",
    "thread_purge_all",
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

const KNOWLEDGE_DEFAULT_OFF: &[&str] = &[
    "people_refresh_address_book",
    "create_workflow",
    "install_workflow_from_url",
    "uninstall_workflow",
    "thread_delete",
    "thread_purge_all",
    "learning_update_facet",
    "learning_pin_facet",
    "learning_unpin_facet",
    "learning_forget_facet",
    "learning_rebuild_cache",
    "learning_reset_cache",
    "learning_save_profile",
    "learning_enrich_profile",
];

const KNOWLEDGE_ALWAYS_ON: &[&str] = &[
    "people_list",
    "people_resolve",
    "list_workflows",
    "list_workflow_runs",
    "thread_list",
    "thread_create",
    "learning_list_facets",
    "learning_cache_stats",
];

#[test]
fn knowledge_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, KNOWLEDGE_TOOLS);
}

#[test]
fn knowledge_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in KNOWLEDGE_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in KNOWLEDGE_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn knowledge_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(
        &mut tools,
        &[
            "people_refresh_address_book".to_string(),
            "workflow_manage".to_string(),
            "thread_destructive".to_string(),
            "learning_manage".to_string(),
        ],
    );
    let names = tool_names(&tools);
    for on in KNOWLEDGE_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
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

#[test]
fn system_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, SYSTEM_TOOLS);
}

#[test]
fn system_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in SYSTEM_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in SYSTEM_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn system_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["service_lifecycle".to_string()]);
    let names = tool_names(&tools);
    for on in SYSTEM_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}

#[tokio::test]
async fn health_system_info_through_registry() {
    let tmp = TempDir::new().unwrap();
    let tools = expansion_tools_for(&tmp);
    let out = find_tool(&tools, "health_system_info")
        .execute(serde_json::json!({}))
        .await
        .expect("health_system_info");
    assert!(out.output_for_llm(false).contains("os"));
}

// ── Theme: Account & money ──────────────────────────────────────────────────

const MONEY_TOOLS: &[&str] = &[
    "referral_get_stats",
    "referral_claim",
    "billing_get_plan",
    "billing_get_balance",
    "billing_list_transactions",
    "billing_get_auto_recharge",
    "billing_list_cards",
    "billing_list_coupons",
    "billing_create_stripe_portal",
    "billing_purchase_plan",
    "billing_top_up_credits",
    "billing_create_coinbase_charge",
    "billing_create_setup_intent",
    "billing_update_card",
    "billing_delete_card",
    "billing_redeem_coupon",
    "billing_update_auto_recharge",
    "team_list",
    "team_get_usage",
    "team_get",
    "team_list_members",
    "team_list_invites",
    "team_create",
    "team_update",
    "team_delete",
    "team_switch",
    "team_join",
    "team_leave",
    "team_create_invite",
    "team_revoke_invite",
    "team_remove_member",
    "team_change_member_role",
    "credential_list",
    "session_state",
    "session_get_user",
    "oauth_connect_url",
    "oauth_list",
];

const MONEY_DEFAULT_OFF: &[&str] = &[
    "billing_purchase_plan",
    "billing_top_up_credits",
    "billing_create_coinbase_charge",
    "billing_create_setup_intent",
    "billing_update_card",
    "billing_delete_card",
    "billing_redeem_coupon",
    "billing_update_auto_recharge",
    "team_create",
    "team_update",
    "team_delete",
    "team_switch",
    "team_join",
    "team_leave",
    "team_create_invite",
    "team_revoke_invite",
    "team_remove_member",
    "team_change_member_role",
];

const MONEY_ALWAYS_ON: &[&str] = &[
    "billing_get_plan",
    "billing_list_cards",
    "team_list",
    "team_get",
    "credential_list",
    "session_state",
    "oauth_list",
    "referral_get_stats",
];

#[test]
fn money_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, MONEY_TOOLS);
}

#[test]
fn money_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in MONEY_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in MONEY_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn money_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(
        &mut tools,
        &["billing_writes".to_string(), "team_admin".to_string()],
    );
    let names = tool_names(&tools);
    for on in MONEY_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}

// ── Theme: Desktop perception, MCP registry, workspace ──────────────────────

const DESKTOP_TOOLS: &[&str] = &[
    "screen_intelligence_status",
    "screen_intelligence_capture_image_ref",
    "screen_intelligence_vision_recent",
    "screen_intelligence_vision_flush",
    "screen_intelligence_refresh_permissions",
    "screen_intelligence_capture_now",
    "screen_intelligence_capture_test",
    "screen_intelligence_session_start",
    "screen_intelligence_session_stop",
    "screen_intelligence_input_action",
    "screen_intelligence_globe_listener_start",
    "screen_intelligence_globe_listener_poll",
    "screen_intelligence_globe_listener_stop",
    "screen_intelligence_request_permissions",
    "screen_intelligence_request_permission",
    "mcp_registry_search",
    "mcp_registry_get",
    "mcp_registry_installed_list",
    "mcp_registry_status",
    "mcp_registry_connect",
    "mcp_registry_disconnect",
    "mcp_registry_tool_call",
    "mcp_registry_config_assist",
    "mcp_registry_install",
    "mcp_registry_uninstall",
    "workspace_read_persona",
    "workspace_update_persona",
    "workspace_reset_persona",
    "workspace_init",
];

const DESKTOP_DEFAULT_OFF: &[&str] = &[
    "screen_intelligence_request_permissions",
    "screen_intelligence_request_permission",
    "mcp_registry_install",
    "mcp_registry_uninstall",
    "workspace_update_persona",
    "workspace_reset_persona",
    "workspace_init",
];

const DESKTOP_ALWAYS_ON: &[&str] = &[
    "screen_intelligence_status",
    "screen_intelligence_capture_now",
    "mcp_registry_search",
    "mcp_registry_tool_call",
    "mcp_registry_connect",
    "workspace_read_persona",
];

#[test]
fn desktop_tools_are_registered() {
    let tmp = TempDir::new().unwrap();
    let names = tool_names(&expansion_tools_for(&tmp));
    assert_contains_all(&names, DESKTOP_TOOLS);
}

#[test]
fn desktop_default_off_tools_are_filtered_when_not_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(&mut tools, &["file_read".to_string()]);
    let names = tool_names(&tools);
    for off in DESKTOP_DEFAULT_OFF {
        assert!(
            !names.iter().any(|n| n == off),
            "default-off tool `{off}` must be filtered out when not opted in; got: {names:?}"
        );
    }
    for on in DESKTOP_ALWAYS_ON {
        assert!(
            names.iter().any(|n| n == on),
            "always-on tool `{on}` must be retained regardless of preferences"
        );
    }
}

#[test]
fn desktop_default_off_tools_retained_when_opted_in() {
    let tmp = TempDir::new().unwrap();
    let mut tools = expansion_tools_for(&tmp);
    filter_tools_by_user_preference(
        &mut tools,
        &[
            "screen_permissions".to_string(),
            "mcp_manage".to_string(),
            "workspace_manage".to_string(),
        ],
    );
    let names = tool_names(&tools);
    for on in DESKTOP_DEFAULT_OFF {
        assert!(
            names.iter().any(|n| n == on),
            "opted-in tool `{on}` must be retained; got: {names:?}"
        );
    }
}
