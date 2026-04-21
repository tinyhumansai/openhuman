use super::*;

use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::config::{Config, DelegateAgentConfig};
use crate::openhuman::memory::Memory;
use crate::openhuman::security::SecurityPolicy;
use std::collections::HashMap;
use std::sync::Arc;

/// Create the default tool registry
pub fn default_tools(security: Arc<SecurityPolicy>) -> Vec<Box<dyn Tool>> {
    default_tools_with_runtime(security, Arc::new(NativeRuntime::new()))
}

/// Create the default tool registry with explicit runtime adapter.
pub fn default_tools_with_runtime(
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
) -> Vec<Box<dyn Tool>> {
    vec![
        Box::new(ShellTool::new(security.clone(), runtime)),
        Box::new(FileReadTool::new(security.clone())),
        Box::new(FileWriteTool::new(security)),
    ]
}

/// Create full tool registry including memory tools and optional Composio
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    memory: Arc<dyn Memory>,
    composio_key: Option<&str>,
    composio_entity_id: Option<&str>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    workspace_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    fallback_api_key: Option<&str>,
    root_config: &crate::openhuman::config::Config,
) -> Vec<Box<dyn Tool>> {
    all_tools_with_runtime(
        config,
        security,
        Arc::new(NativeRuntime::new()),
        memory,
        composio_key,
        composio_entity_id,
        browser_config,
        http_config,
        workspace_dir,
        agents,
        fallback_api_key,
        root_config,
    )
}

/// Create full tool registry including memory tools and optional Composio.
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    memory: Arc<dyn Memory>,
    composio_key: Option<&str>,
    composio_entity_id: Option<&str>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    workspace_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    fallback_api_key: Option<&str>,
    root_config: &crate::openhuman::config::Config,
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ShellTool::new(security.clone(), runtime)),
        Box::new(FileReadTool::new(security.clone())),
        Box::new(FileWriteTool::new(security.clone())),
        Box::new(CsvExportTool::new(security.clone())),
        // Sub-agent dispatch — lets the parent agent delegate focused
        // sub-tasks (research, code execution, API specialists, …) by
        // calling `spawn_subagent { agent_id, prompt, … }`. The runner
        // builds a narrow Agent from an `AgentDefinition` lookup and
        // returns a single text result. See
        // `agent::harness::subagent_runner` for the dispatch path.
        Box::new(SpawnSubagentTool::new()),
        Box::new(CompleteOnboardingTool::new()),
        Box::new(CurrentTimeTool::new()),
        Box::new(CronAddTool::new(config.clone(), security.clone())),
        Box::new(CronListTool::new(config.clone())),
        Box::new(CronRemoveTool::new(config.clone())),
        Box::new(CronUpdateTool::new(config.clone(), security.clone())),
        Box::new(CronRunTool::new(config.clone())),
        Box::new(CronRunsTool::new(config.clone())),
        Box::new(MemoryStoreTool::new(memory.clone(), security.clone())),
        Box::new(MemoryRecallTool::new(memory.clone())),
        Box::new(MemoryForgetTool::new(memory.clone(), security.clone())),
        Box::new(ScheduleTool::new(security.clone(), root_config.clone())),
        Box::new(ProxyConfigTool::new(config.clone(), security.clone())),
        Box::new(GitOperationsTool::new(
            security.clone(),
            workspace_dir.to_path_buf(),
        )),
        Box::new(PushoverTool::new(
            security.clone(),
            workspace_dir.to_path_buf(),
        )),
    ];

    if browser_config.enabled {
        // Add legacy browser_open tool for simple URL opening
        tools.push(Box::new(BrowserOpenTool::new(
            security.clone(),
            browser_config.allowed_domains.clone(),
        )));
        // Add full browser automation tool (pluggable backend)
        tools.push(Box::new(BrowserTool::new_with_backend(
            security.clone(),
            browser_config.allowed_domains.clone(),
            browser_config.session_name.clone(),
            browser_config.backend.clone(),
            browser_config.native_headless,
            browser_config.native_webdriver_url.clone(),
            browser_config.native_chrome_path.clone(),
            ComputerUseConfig {
                endpoint: browser_config.computer_use.endpoint.clone(),
                api_key: browser_config.computer_use.api_key.clone(),
                timeout_ms: browser_config.computer_use.timeout_ms,
                allow_remote_endpoint: browser_config.computer_use.allow_remote_endpoint,
                window_allowlist: browser_config.computer_use.window_allowlist.clone(),
                max_coordinate_x: browser_config.computer_use.max_coordinate_x,
                max_coordinate_y: browser_config.computer_use.max_coordinate_y,
            },
        )));
    }

    // HTTP request — always registered. `http_request.allowed_domains`
    // + `security` still gate which hosts are reachable; there is no
    // enable flag because every session needs basic HTTP as a baseline
    // capability.
    tools.push(Box::new(HttpRequestTool::new(
        security.clone(),
        http_config.allowed_domains.clone(),
        http_config.max_response_size,
        http_config.timeout_secs,
    )));

    // Web search — always registered. Provider / API-key / budget
    // knobs still come from `config.web_search`, but there is no
    // enable flag: every session needs research as a baseline
    // capability.
    tools.push(Box::new(WebSearchTool::new(
        root_config.web_search.provider.clone(),
        root_config.web_search.brave_api_key.clone(),
        root_config.web_search.parallel_api_key.clone(),
        root_config.web_search.max_results,
        root_config.web_search.timeout_secs,
    )));

    // Vision tools are always available
    tools.push(Box::new(ScreenshotTool::new(security.clone())));
    tools.push(Box::new(ImageInfoTool::new(security.clone())));

    // Native mouse + keyboard control (disabled by default)
    if root_config.computer_control.enabled {
        tools.push(Box::new(MouseTool::new(security.clone())));
        tools.push(Box::new(KeyboardTool::new(security.clone())));
        tracing::debug!("[computer] mouse and keyboard tools registered");
    }

    if let Some(key) = composio_key {
        if !key.is_empty() {
            tools.push(Box::new(ComposioTool::new(
                key,
                composio_entity_id,
                security.clone(),
            )));
        }
    }

    // Tool effectiveness stats (enabled when learning is on)
    tracing::debug!(
        learning_enabled = root_config.learning.enabled,
        tool_tracking_enabled = root_config.learning.tool_tracking_enabled,
        "evaluating ToolStatsTool registration"
    );
    if root_config.learning.enabled && root_config.learning.tool_tracking_enabled {
        tools.push(Box::new(ToolStatsTool::new(memory.clone())));
        tracing::debug!("ToolStatsTool registered");
    }

    // Add delegation tool when agents are configured
    if !agents.is_empty() {
        let delegate_agents: HashMap<String, DelegateAgentConfig> = agents
            .iter()
            .map(|(name, cfg)| (name.clone(), cfg.clone()))
            .collect();
        let delegate_fallback_credential = fallback_api_key.and_then(|value| {
            let trimmed_value = value.trim();
            (!trimmed_value.is_empty()).then(|| trimmed_value.to_owned())
        });
        tools.push(Box::new(DelegateTool::new_with_options(
            delegate_agents,
            delegate_fallback_credential,
            security.clone(),
            crate::openhuman::providers::ProviderRuntimeOptions {
                auth_profile_override: None,
                openhuman_dir: root_config
                    .config_path
                    .parent()
                    .map(std::path::PathBuf::from),
                secrets_encrypt: root_config.secrets.encrypt,
                reasoning_enabled: root_config.runtime.reasoning_enabled,
            },
        )));
    }

    // ── Agent integration tools (backend-proxied) ─────────────────
    if let Some(client) = crate::openhuman::integrations::build_client(root_config) {
        tracing::debug!("[integrations] client built successfully");
        if root_config.integrations.apify.enabled {
            tools.push(Box::new(
                crate::openhuman::integrations::ApifyRunActorTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ApifyGetRunStatusTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ApifyGetRunResultsTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered apify tools");
        } else {
            tracing::debug!("[integrations] apify disabled — skipping");
        }
        if root_config.integrations.google_places.enabled {
            tools.push(Box::new(
                crate::openhuman::integrations::GooglePlacesSearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::GooglePlacesDetailsTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered google_places tools");
        } else {
            tracing::debug!("[integrations] google_places disabled — skipping");
        }
        if root_config.integrations.parallel.enabled {
            tools.push(Box::new(
                crate::openhuman::integrations::ParallelSearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ParallelExtractTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered parallel tools");
        } else {
            tracing::debug!("[integrations] parallel disabled — skipping");
        }
        if root_config.integrations.twilio.enabled {
            tools.push(Box::new(
                crate::openhuman::integrations::TwilioCallTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered twilio tools");
        } else {
            tracing::debug!("[integrations] twilio disabled — skipping");
        }

        // Composio — backend-proxied 1000+ OAuth integrations. Registers
        // five agent tools (list_toolkits, list_connections, authorize,
        // list_tools, execute) when the composio toggle is on. See
        // `src/openhuman/composio/tools.rs` for per-tool details.
        let composio_tools = crate::openhuman::composio::all_composio_agent_tools(root_config);
        if !composio_tools.is_empty() {
            tracing::debug!(
                count = composio_tools.len(),
                "[integrations] registered composio tools"
            );
            tools.extend(composio_tools);
        } else {
            tracing::debug!("[integrations] composio disabled — skipping");
        }
    } else {
        tracing::debug!(
            "[integrations] build_client returned None — integration tools not registered"
        );
    }

    tools
}

#[cfg(test)]
mod tests {
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

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
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

        let browser = BrowserConfig::default();
        let http = crate::openhuman::config::HttpRequestConfig::default();
        let cfg = test_config(&tmp);

        let tools = all_tools(
            Arc::new(Config::default()),
            &security,
            mem,
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

        let browser = BrowserConfig::default();
        let http = crate::openhuman::config::HttpRequestConfig::default();
        let cfg = test_config(&tmp);

        let tools = all_tools(
            Arc::new(Config::default()),
            &security,
            mem,
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

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
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

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
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

        let browser = BrowserConfig::default();
        let http = crate::openhuman::config::HttpRequestConfig::default();
        let cfg = test_config(&tmp);

        let mut agents = HashMap::new();
        agents.insert(
            "researcher".to_string(),
            DelegateAgentConfig {
                model: "llama3".to_string(),
                system_prompt: None,
                api_key: None,
                temperature: None,
                max_depth: 3,
            },
        );

        let tools = all_tools(
            Arc::new(Config::default()),
            &security,
            mem,
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &agents,
            Some("delegate-test-credential"),
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

        let browser = BrowserConfig::default();
        let http = crate::openhuman::config::HttpRequestConfig::default();
        let cfg = test_config(&tmp);

        let tools = all_tools(
            Arc::new(Config::default()),
            &security,
            mem,
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
            &cfg,
        );
        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(!names.contains(&"delegate"));
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

        let browser = BrowserConfig::default();
        let http = crate::openhuman::config::HttpRequestConfig::default();
        let cfg = test_config(&tmp);

        // Default config has computer_control.enabled = false
        let tools = all_tools(
            Arc::new(Config::default()),
            &security,
            mem,
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
            Arc::from(crate::openhuman::memory::create_memory(&mem_cfg, tmp.path(), None).unwrap());

        let browser = BrowserConfig::default();
        let http = crate::openhuman::config::HttpRequestConfig::default();
        let mut cfg = test_config(&tmp);
        cfg.computer_control.enabled = true;

        let tools = all_tools(
            Arc::new(Config::default()),
            &security,
            mem,
            None,
            None,
            &browser,
            &http,
            tmp.path(),
            &HashMap::new(),
            None,
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
}
