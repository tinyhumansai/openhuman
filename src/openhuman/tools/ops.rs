use super::*;

use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::config::{Config, DelegateAgentConfig};
use crate::openhuman::memory::Memory;
use crate::openhuman::node_runtime::NodeBootstrap;
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

/// Create full tool registry including memory tools.
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    memory: Arc<dyn Memory>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    workspace_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::openhuman::config::Config,
) -> Vec<Box<dyn Tool>> {
    all_tools_with_runtime(
        config,
        security,
        Arc::new(NativeRuntime::new()),
        memory,
        browser_config,
        http_config,
        workspace_dir,
        agents,
        root_config,
    )
}

/// Create full tool registry including memory tools.
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    memory: Arc<dyn Memory>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    workspace_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::openhuman::config::Config,
) -> Vec<Box<dyn Tool>> {
    // Build a session-scoped managed Node.js bootstrap once, so ShellTool,
    // NodeExecTool, and NpmExecTool all share the same memoised resolution
    // state. Disabled when `node.enabled = false` — in that case shell skips
    // PATH injection and node/npm tools are not registered.
    let node_bootstrap: Option<Arc<NodeBootstrap>> = if root_config.node.enabled {
        tracing::debug!(
            version = %root_config.node.version,
            prefer_system = root_config.node.prefer_system,
            "[tools::ops] node runtime enabled — constructing shared NodeBootstrap"
        );
        Some(Arc::new(NodeBootstrap::new(
            root_config.node.clone(),
            workspace_dir.to_path_buf(),
            reqwest::Client::new(),
        )))
    } else {
        tracing::debug!(
            "[tools::ops] node runtime disabled — shell PATH injection + node_exec/npm_exec suppressed"
        );
        None
    };

    let shell: Box<dyn Tool> = if let Some(bootstrap) = node_bootstrap.as_ref() {
        Box::new(ShellTool::with_node_bootstrap(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
        ))
    } else {
        Box::new(ShellTool::new(security.clone(), Arc::clone(&runtime)))
    };

    let mut tools: Vec<Box<dyn Tool>> = vec![
        shell,
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
        Box::new(CheckOnboardingStatusTool::new()),
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
                api_key: None,
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

    // curl — always registered. Shares `http_request.allowed_domains`,
    // adds streaming-to-disk with a hard byte ceiling. Writes land
    // under `<workspace>/<curl.dest_subdir>`.
    tools.push(Box::new(CurlTool::new(
        security.clone(),
        http_config.allowed_domains.clone(),
        workspace_dir.to_path_buf(),
        root_config.curl.dest_subdir.clone(),
        root_config.curl.max_download_bytes,
        root_config.curl.timeout_secs,
    )));

    // gitbooks — answers questions about OpenHuman by calling the
    // GitBook MCP server. Two tools mirroring the upstream MCP tools.
    if root_config.gitbooks.enabled {
        tools.push(Box::new(GitbooksSearchTool::new(
            root_config.gitbooks.endpoint.clone(),
            root_config.gitbooks.timeout_secs,
        )));
        tools.push(Box::new(GitbooksGetPageTool::new(
            root_config.gitbooks.endpoint.clone(),
            root_config.gitbooks.timeout_secs,
        )));
        tracing::debug!("[gitbooks] registered gitbooks_search + gitbooks_get_page");
    }

    // Web search — always registered. Result/timeout budget
    // knobs still come from `config.web_search`, but there is no
    // enable flag: every session needs research as a baseline
    // capability.
    tools.push(Box::new(WebSearchTool::new(
        crate::openhuman::integrations::build_client(root_config),
        root_config.web_search.max_results,
        root_config.web_search.timeout_secs,
    )));

    // Managed Node.js exec tools — gated on `root_config.node.enabled`.
    // Both share the same `NodeBootstrap` as ShellTool so the download +
    // extract + install pipeline runs at most once per session.
    if let Some(bootstrap) = node_bootstrap.as_ref() {
        tools.push(Box::new(NodeExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
        )));
        tools.push(Box::new(NpmExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
        )));
        tracing::debug!("[tools::ops] registered node_exec + npm_exec");
    }

    // Vision tools are always available
    tools.push(Box::new(ScreenshotTool::new(security.clone())));
    tools.push(Box::new(ImageInfoTool::new(security.clone())));

    // Native mouse + keyboard control (disabled by default)
    if root_config.computer_control.enabled {
        tools.push(Box::new(MouseTool::new(security.clone())));
        tools.push(Box::new(KeyboardTool::new(security.clone())));
        tracing::debug!("[computer] mouse and keyboard tools registered");
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
        tools.push(Box::new(DelegateTool::new_with_options(
            delegate_agents,
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
            tools.push(Box::new(
                crate::openhuman::integrations::ParallelChatTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ParallelResearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ParallelEnrichTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ParallelDatasetTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered parallel tools");
        } else {
            tracing::debug!("[integrations] parallel disabled — skipping");
        }
        if root_config.integrations.stock_prices.enabled {
            tools.push(Box::new(
                crate::openhuman::integrations::StockQuoteTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockExchangeRateTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockOptionsTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockCryptoSeriesTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockCommodityTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered stock_prices tools");
        } else {
            tracing::debug!("[integrations] stock_prices disabled — skipping");
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
}
