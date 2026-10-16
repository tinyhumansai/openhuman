use super::*;
use crate::config::DelegateAgentConfig;
use crate::tools::ops::default_tools;

#[test]
fn all_tools_includes_browser_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());

    let browser = BrowserConfig {
        enabled: true,
        allowed_domains: vec!["example.com".into()],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::config::HttpRequestConfig::default();
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
    // The browser tools are the `modules` gate's (tools/ops.rs): present when
    // it is compiled in, absent in a gates-off build.
    assert_eq!(
        names.contains(&"browser_open"),
        cfg!(feature = "modules"),
        "browser_open follows the `modules` gate"
    );
    assert!(names.contains(&"pushover"));
    assert!(names.contains(&"proxy_config"));
}

#[test]
fn computer_component_off_unregisters_desktop_and_browser_tools() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: true,
        allowed_domains: vec!["example.com".into()],
        session_name: None,
        ..BrowserConfig::default()
    };
    let http = crate::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);
    let mut root = Config::default();
    root.components.computer = false;

    let tools = all_tools(
        Arc::new(root),
        &security,
        AuditLogger::disabled(),
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(!names.contains(&"browser_open"), "{names:?}");
    assert!(!names.contains(&"browser"), "{names:?}");
    assert!(
        !names.iter().any(|name| name.starts_with("desktop_")),
        "{names:?}"
    );
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
fn all_tools_includes_delegate_when_agents_configured() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());

    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
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

    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
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
fn all_tools_registers_no_hosted_integration_families_without_a_tool_source() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.api_url = Some("https://backend.example.test".to_string());
    cfg.integrations.google_places.enabled = true;
    cfg.integrations.parallel.enabled = true;
    cfg.integrations.tinyfish.enabled = true;
    cfg.integrations.stock_prices.enabled = true;
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

    // The hosted families (Google Places, stock data, file storage, managed
    // media) come from a host's registered tool source, so a bare core offers
    // none of them even when enabled and signed in.
    for hosted in [
        "google_places_search",
        "google_places_details",
        "stock_quote",
        "stock_exchange_rate",
        "stock_options",
        "stock_crypto_series",
        "stock_commodity",
        "storage_upload_file",
        "media_generate_image",
    ] {
        assert!(
            !names.iter().any(|n| n == hosted),
            "`{hosted}` must come from a tool source, not the core; got: {names:?}"
        );
    }
    // Composio's backend mode is a route a host registers; a bare core has
    // none, so a session alone does not register the composio tools.
    assert!(
        !names.iter().any(|n| n.starts_with("composio_")),
        "composio backend mode needs a registered route; got: {names:?}"
    );
    // The backend retired its Twilio route; the agent must not be offered a
    // tool that can only fail.
    assert!(
        !names.iter().any(|n| n == "twilio_call"),
        "twilio_call must no longer be registered"
    );
}

#[test]
fn all_tools_registers_brave_engine_and_lsp_when_enabled() {
    // Search registers one tool per capability role from the TinySearch
    // catalog; a usable BYOK Brave key yields the `search` role tool
    // alongside lsp.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.search.providers.insert(
        "brave".into(),
        crate::config::SearchProviderSettings::direct(),
    );
    cfg.search.brave.api_key = Some("test-brave-key".into());

    let _env_guard = crate::config::TEST_ENV_LOCK.blocking_lock();
    unsafe {
        std::env::set_var(crate::tools::implementations::LSP_ENABLED_ENV, "1");
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
            #[cfg(feature = "modules")]
            "web_search_tool",
            "lsp",
        ],
    );

    unsafe {
        std::env::remove_var(crate::tools::implementations::LSP_ENABLED_ENV);
    }
}

#[test]
fn all_tools_registers_querit_as_the_search_role_when_enabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.search.providers.insert(
        "querit".into(),
        crate::config::SearchProviderSettings::direct(),
    );
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
    #[cfg(feature = "modules")]
    assert_contains_all(&names, &["web_search_tool"]);
    // Provider tools stay behind the role tool in the default presentation.
    assert!(
        !names.iter().any(|name| name == "querit_search"),
        "{names:?}"
    );
}

#[test]
fn all_tools_omits_search_surface_when_search_is_disabled() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    cfg.api_url = Some("https://backend.example.test".to_string());
    cfg.search.enabled = Some(false);
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
        "web_answer_tool",
        "web_contents_tool",
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
