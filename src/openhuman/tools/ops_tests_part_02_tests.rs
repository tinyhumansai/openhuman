use super::*;

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
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

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
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

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
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
#[cfg(feature = "runtime-node")]
fn all_tools_registers_node_exec_when_node_enabled() {
    // Default NodeConfig has `enabled = true`, so both `node_exec` and
    // `npm_exec` must appear in the registry. Regression guard for the
    // skills integration — if this fires, managed-node skills silently
    // lose both tools.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
fn all_tools_registers_python_exec_when_python_enabled() {
    // Default RuntimePythonConfig has `enabled = true`, so `python_exec` must
    // appear in the registry (routes inline code through the runtime pool, #5106).
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"python_exec"),
        "python_exec must be registered when runtime_python.enabled=true; got: {names:?}"
    );
}

#[test]
fn all_tools_excludes_node_exec_when_node_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.node.enabled = false;

    let tools = all_tools(
        Arc::new(Config::default()),
        &security,
        AuditLogger::disabled(),
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
fn all_tools_registers_integration_families_when_enabled_and_signed_in() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.api_url = Some("https://backend.example.test".to_string());
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
    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.search.engine = crate::openhuman::config::SEARCH_ENGINE_QUERIT.into();
    cfg.search.querit.api_key = Some("test-querit-key".into());

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
    assert_contains_all(&names, &["web_search_tool", "querit_search"]);
}

#[test]
fn all_tools_omits_search_surface_when_search_is_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
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
