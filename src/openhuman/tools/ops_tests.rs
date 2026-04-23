use super::*;
use crate::openhuman::config::{BrowserConfig, Config, MemoryConfig};
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    Config {
        workspace_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    }
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
