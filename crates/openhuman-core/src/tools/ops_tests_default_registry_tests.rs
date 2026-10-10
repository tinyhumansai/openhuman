use super::*;
#[test]
fn all_tools_registers_the_memory_tool_while_memory_is_on() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let cfg = test_config(&tmp);
    crate::memory::test_fixtures::bind_reference(&cfg);
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
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

    let memory = tools
        .iter()
        .find(|tool| tool.name() == crate::memory::tools::MEMORY_TOOL_NAME)
        .expect("the memory tool must be registered while memory is on");
    assert_eq!(
        tool_group(memory.name()),
        crate::core::all::DomainGroup::Memory
    );
    // The search half of deferral is the harness's intrinsic bridge, never a
    // registered tool: a host `tool_search` would shadow it.
    assert!(!tools
        .iter()
        .any(|tool| { tool.name() == crate::tools::implementations::meta::TOOL_SEARCH_NAME }));
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
fn every_packed_tool_name_resolves_to_a_registered_tool() {
    // A pack advertises a menu. `render_pack_filtered` skips a name it cannot
    // resolve (tools.rs "a pack may name a tool this build compiled out"), so a
    // stale entry does not error — the listing is just quietly short, and any
    // prompt that instructs the agent to call it describes a tool that will
    // never appear in its schema. The `crypto` pack carried five such names
    // (`wallet_balances`, `wallet_network_defaults`, `wallet_supported_assets`,
    // `wallet_encode_erc20_transfer`, `wallet_execute_prepared`) whose backing
    // functions exist only as `wallet.*` RPC methods, never as agent tools.
    //
    // Packs whose tools sit behind a non-default Cargo feature are skipped
    // entirely — see `pack_is_assertable` below. Within an asserted pack every
    // name must resolve.
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
        allowed_domains: vec![],
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
    let registered: std::collections::HashSet<&str> = tools.iter().map(|t| t.name()).collect();

    // Delegation tools are synthesised per-session from each agent's
    // `delegate_name`, not built by `all_tools`, so they are legitimately
    // absent here. `owners` holds agent ids (`crypto_agent`); the names that
    // appear in `tools` are delegate names (`do_crypto`), so resolve them from
    // the agent registry rather than from `owners`.
    //
    // `.expect`, not `.unwrap_or_default()`: an empty delegate set would make
    // EVERY pack delegate (`do_crypto`, `build_workflow`, `run_skill`, …) report
    // as missing, so a registry that failed to load would surface as a list of
    // phantom names — the exact failure this test exists to report, raised for
    // the wrong reason. `load_builtins` documents that built-in TOML is baked
    // into the binary and must always parse, so an `Err` here is a broken
    // invariant worth failing loudly on rather than absorbing.
    let delegates: std::collections::HashSet<String> =
        crate::agent::registry::agents::load_builtins()
            .expect("built-in agent registry must load; without it every pack delegate would be reported as a phantom name")
            .into_iter()
            .filter_map(|d| d.delegate_name)
            .collect();

    // `PACKS` is unconditional, but most packs' tools are behind Cargo features
    // that are NOT in `default` — `flows`, `mcp`, `skills`, `web3`, `documents`,
    // `voice`. Under a partial feature set "missing" means "compiled out", not
    // "stale", and asserting there reports the pack's REAL tools as missing.
    // That is what the `Rust Feature-Gate Smoke (gates off)` lane caught twice.
    //
    // A stale name is a property of the `PACKS` table, not of the build, so
    // checking it in one fully-featured configuration is sufficient. The product
    // lane runs with all of these on, which is where a stale entry is caught.
    //
    // Deliberately ONE condition rather than a per-pack feature map: a map has
    // to be updated every time a pack becomes gated, and the failure mode of
    // forgetting is a confusing red in an unrelated lane rather than a clear
    // signal here.
    if !cfg!(all(
        feature = "flows",
        feature = "mcp",
        feature = "skills",
        feature = "web3",
        feature = "documents",
        feature = "voice",
    )) {
        return;
    }

    let mut missing: Vec<String> = Vec::new();
    for pack in crate::tools::toolpacks::PACKS {
        // Registered only once the user is signed in: `composio`'s tools come
        // from `all_composio_agent_tools`, which returns an empty vec without
        // a session (`integrations/composio/tools/registry.rs`), and
        // `storage` / `media` are built behind `integrations::build_client`,
        // which needs a session token (`file_storage/tools/registry.rs`,
        // `media/generation/tools.rs`). Runtime auth state no unit test can
        // satisfy, in any feature configuration. Every name in those three is
        // a real tool in the source it is built from, verified by hand at the
        // time each pack was added — that is the check this skip replaces.
        if matches!(pack.id, "composio" | "storage" | "media") {
            continue;
        }
        for name in pack.tools {
            if registered.contains(name) || delegates.contains(*name) {
                continue;
            }
            missing.push(format!("{}::{}", pack.id, name));
        }
    }
    assert!(
        missing.is_empty(),
        "every tool named in a pack must resolve to a registered tool (or be a \
         delegate name); these do not: {missing:?}"
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
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
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

/// The `docs` MCP server the registry-gate tests declare in config.
fn docs_server() -> crate::config::McpServerConfig {
    crate::config::McpServerConfig::from(tinymcp_bus::McpServerConfig {
        name: "docs".into(),
        endpoint: "https://example.com/mcp".into(),
        description: Some("Example docs MCP".into()),
        ..Default::default()
    })
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
    cfg.mcp_client.servers.push(docs_server());

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
/// (`mcp_registry_*`).
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
    cfg.mcp_client.servers.push(docs_server());

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
    let browser = BrowserConfig::default();
    let http = crate::config::HttpRequestConfig::default();
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
fn all_tools_default_registry_contains_expected_baseline_surface() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
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
    let names = tool_names(&tools);

    let expected = vec![
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
        "image_info",
    ];
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

    let browser = BrowserConfig {
        enabled: false,
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
    assert!(!names.contains(&"browser_open"));
    assert!(names.contains(&"schedule"));
    assert!(names.contains(&"pushover"));
    assert!(names.contains(&"proxy_config"));
}

/// #5505: the producers write into the folder chosen in Settings, not a
/// hard-coded default. Generation itself needs the tinydocs module, which a
/// unit test does not have, but the artifact is reserved in the files folder
/// before generation runs, so its record names the folder either way.
#[cfg(feature = "documents")]
#[tokio::test]
async fn document_tool_writes_into_the_configured_files_folder() {
    let tmp = TempDir::new().unwrap();
    let security = Arc::new(SecurityPolicy::default());
    let browser = BrowserConfig {
        enabled: false,
        ..BrowserConfig::default()
    };
    let http = crate::config::HttpRequestConfig::default();
    let mut cfg = test_config(&tmp);
    let chosen = tmp.path().join("Chosen");
    cfg.files_dir_override = Some(chosen.clone());
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
    let tool = tools
        .iter()
        .find(|t| t.name() == "generate_document")
        .expect("generate_document registered");

    let _ = tool
        .execute(serde_json::json!({
            "title": "Charter",
            "sections": [{ "heading": "Overview", "paragraphs": ["In brief."] }]
        }))
        .await;

    let (artifacts, _) =
        crate::agent::artifacts::store::list_artifacts(&cfg.workspace_dir, 0, 10, None)
            .await
            .expect("list artifacts");
    assert_eq!(artifacts.len(), 1, "{artifacts:?}");
    assert_eq!(
        artifacts[0].file_root.as_deref(),
        Some(chosen.to_string_lossy().as_ref())
    );
}
