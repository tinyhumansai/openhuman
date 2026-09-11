use super::*;

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
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

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

/// The three `whatsapp_data_*` agent tools are gone, in every build.
///
/// They queried a shell-side SQLite store whose only writer was the CDP
/// `whatsapp_scanner`, deleted in #5478 when the app moved off Chromium — so
/// from that release the tools read a store nothing could write. This asserts
/// the removal in both directions of the `channels` gate at once, replacing the
/// present/absent pair that used to pin them.
#[test]
fn whatsapp_data_tools_are_gone_in_every_build() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
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
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    for absent in [
        "whatsapp_data_list_chats",
        "whatsapp_data_list_messages",
        "whatsapp_data_search_messages",
    ] {
        assert!(
            !names.iter().any(|n| n == absent),
            "`{absent}` was removed with the store it read; got: {names:?}"
        );
    }
}

#[test]
fn all_tools_includes_spawn_async_subagent() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
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
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };
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
    // The embedding seam fails loudly when unwired. This
    // test doesn't use that helper (it needs the `Arc<dyn Memory>` alongside
    // its own config setup below), so it installs the seams directly.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

    let browser = BrowserConfig::default();
    let http = crate::openhuman::config::HttpRequestConfig::default();
    let cfg = test_config(&tmp);

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
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"curl"),
        "curl must always be registered; got: {names:?}"
    );
}

// Compile-time `media` feature gate (#4804). The media-generation agent tools
// (`media_generate_*`) are present only when the `media` feature is compiled
// in AND an integration client is configured. The disabled build proves the
// module + its single call site drop out entirely (leaf gate, no stub facade).
#[cfg(feature = "media")]
#[test]
fn media_tools_registered_when_feature_on() {
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, "http://127.0.0.1:1");
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        names.contains(&"media_generate_image"),
        "media tools must register with the `media` feature on + an integration \
         client; got: {names:?}"
    );
}

#[cfg(not(feature = "media"))]
#[test]
fn media_tools_absent_when_feature_off() {
    let tmp = TempDir::new().unwrap();
    let cfg = integration_test_config(&tmp, "http://127.0.0.1:1");
    store_test_session_token(&cfg);
    let tools = integration_tools_for_config(&tmp, &cfg);
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert!(
        !names.iter().any(|n| n.starts_with("media_")),
        "no `media_*` tools may be registered when the `media` feature is off; \
         got: {names:?}"
    );
}

// Compile-time `documents` feature gate (#5048). The office-document agent
// tools (`generate_presentation`, `generate_document`) are present only when
// the `documents` feature is compiled in — leaf gate, no stub facade, so the
// disabled build must drop both from the tool list entirely.
#[cfg(feature = "documents")]
#[test]
fn document_tools_registered_when_feature_on() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
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
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    assert!(
        names.iter().any(|n| n == "generate_presentation"),
        "generate_presentation must register with `documents` on; got: {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "generate_document"),
        "generate_document must register with `documents` on; got: {names:?}"
    );
}

#[cfg(not(feature = "documents"))]
#[test]
fn document_tools_absent_when_feature_off() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
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
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);
    assert!(
        !names
            .iter()
            .any(|n| n == "generate_presentation" || n == "generate_document"),
        "no document tools may register when the `documents` feature is off; got: {names:?}"
    );
}

#[test]
fn all_tools_registers_gitbooks_when_enabled() {
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
    cfg.gitbooks.enabled = true;

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
// Wholly about the static MCP bridge surface, which the `mcp` feature compiles
// out — no meaningful residue to assert in the disabled build (the
// "no MCP tools registered" direction is covered by
// `all_tools_omits_mcp_tools_when_gate_off` below).
#[cfg(feature = "mcp")]
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

/// The disabled direction of the `mcp` gate (#4799): even with MCP servers
/// declared in config, a build without the `mcp` feature registers NO MCP tool
/// of any family — neither the static bridge (`mcp_*`), the dynamic registry
/// (`mcp_registry_*`), nor the setup-agent surface (`mcp_setup_*`).
///
/// Deliberately asserts by prefix rather than naming the ~19 tools: a new MCP
/// tool added later must not be able to leak into slim builds just because
/// nobody remembered to extend a hardcoded list here.
#[test]
#[cfg(not(feature = "mcp"))]
fn all_tools_omits_mcp_tools_when_gate_off() {
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

    let names = tool_names(&integration_tools_for_config(&tmp, &cfg));
    let leaked: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with("mcp_") || n.starts_with("mcp_registry_"))
        .collect();

    assert!(
        leaked.is_empty(),
        "no MCP tool may be registered when the `mcp` feature is compiled out, \
         even with `[[mcp_client.servers]]` declared in config; leaked: {leaked:?}"
    );
}

#[test]
fn all_tools_skips_gitbooks_when_disabled() {
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
    cfg.gitbooks.enabled = false;

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
        names.contains(&"current_time"),
        "current_time must be registered in the default tool list; got: {names:?}"
    );
}

#[test]
fn all_tools_default_registry_contains_expected_baseline_surface() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
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
        &browser,
        &http,
        tmp.path(),
        &HashMap::new(),
        &cfg,
    );
    let names = tool_names(&tools);

    let mut expected = vec![
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
        "ask_user_clarification",
        "read_workspace_state",
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
        "image_info",
    ];
    // Managed Node tools exist only when the runtime is compiled in — same
    // shape as the `channels` conditional just below.
    if cfg!(feature = "runtime-node") {
        expected.extend(&["node_exec", "npm_exec"]);
    }
    assert_contains_all(&names, &expected);
}

#[test]
fn all_tools_default_registry_has_no_duplicate_tool_names() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
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
    // The embedding seam fails loudly when unwired.
    let _mem_cfg = MemoryConfig {
        backend: "markdown".into(),
        ..MemoryConfig::default()
    };

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
