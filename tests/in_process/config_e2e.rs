//! In-process JSON-RPC E2E coverage for config.

use crate::config_auth_harness::*;

#[test]
fn config_schema_defaults_cover_dashboard_capability_memory_and_security_shapes() {
    let capability = CapabilityProviderConfig::default();
    assert_eq!(
        capability.trust_state,
        CapabilityProviderTrustState::Untrusted
    );
    assert!(!capability.enabled);
    let trusted_capability: CapabilityProviderConfig = serde_json::from_value(json!({
        "id": "external-mcp",
        "display_name": "External MCP",
        "source_uri": "https://example.test/catalog.json",
        "source_digest": "sha256:abc123",
        "trust_state": "trusted",
        "enabled": true
    }))
    .expect("capability provider config should deserialize");
    assert_eq!(
        trusted_capability.trust_state,
        CapabilityProviderTrustState::Trusted
    );

    let dashboard = DashboardConfig::default();
    assert!(dashboard.event_stream.enabled);
    assert_eq!(dashboard.event_stream.max_entries, 200);
    assert_eq!(dashboard.event_stream.new_entries, "top");
    assert!(dashboard.model_health.enabled);
    assert_eq!(dashboard.model_health.min_tasks_for_rating, 10);
    assert_eq!(dashboard.model_health.evaluation_window_tasks, 50);
    assert!(dashboard.diagram_viewer.enabled);
    assert_eq!(
        dashboard.diagram_viewer.source_url,
        "http://localhost:8787/workspace/diagrams/latest.png"
    );
    assert_eq!(dashboard.diagram_viewer.refresh_interval_seconds, 10);
    let partial_dashboard: DashboardConfig = serde_json::from_value(json!({
        "event_stream": {},
        "model_health": {},
        "diagram_viewer": {}
    }))
    .expect("partial dashboard config should fill serde defaults");
    assert!(partial_dashboard.event_stream.enabled);
    assert_eq!(partial_dashboard.model_health.hallucination_threshold, 0.10);
    assert_eq!(
        partial_dashboard.diagram_viewer.refresh_interval_seconds,
        10
    );
    let event_stream: EventStreamConfig =
        serde_json::from_value(json!({})).expect("event stream defaults");
    assert_eq!(event_stream.new_entries, "top");
    let model_health: ModelHealthConfig =
        serde_json::from_value(json!({})).expect("model health defaults");
    assert_eq!(model_health.evaluation_window_tasks, 50);

    let memory = MemoryConfig::default();
    assert_eq!(memory.engine, "tinyhumans");
    assert!(memory.conversations.enabled);
    assert!(memory.recall.enabled);

    let telegram: TelegramConfig = serde_json::from_value(json!({
        "bot_token": "bot-token",
        "allowed_users": ["alice"]
    }))
    .expect("telegram serde defaults");
    assert_eq!(telegram.draft_update_interval_ms, 1000);
    assert!(telegram.silent_streaming);
    assert!(!telegram.mention_only);

    let sandbox = SandboxConfig::default();
    assert!(sandbox.enabled.is_none());
    assert!(sandbox.firejail_args.is_empty());
    let security = SecurityConfig::default();
    assert!(security.audit.enabled);
    let audit = AuditConfig::default();
    assert_eq!(audit.log_path, "audit.log");
    assert_eq!(audit.max_size_mb, 100);

    let observability: openhuman_core::config::schema::ObservabilityConfig =
        serde_json::from_value(json!({})).expect("observability defaults");
    assert!(observability.analytics_enabled);
    assert!(observability.sentry_dsn.is_none());
    let scheduler_gate: openhuman_core::config::schema::SchedulerGateConfig =
        serde_json::from_value(json!({})).expect("scheduler gate defaults");
    assert_eq!(
        scheduler_gate.mode,
        openhuman_core::config::schema::SchedulerGateMode::Auto
    );
    assert_eq!(
        openhuman_core::config::schema::SchedulerGateMode::AlwaysOn.as_str(),
        "always_on"
    );
    assert_eq!(
        openhuman_core::config::schema::SchedulerGateMode::Off.as_str(),
        "off"
    );

    let multimodal = openhuman_core::config::schema::MultimodalConfig {
        max_images: 99,
        max_image_size_mb: 0,
        allow_remote_fetch: true,
    };
    assert_eq!(multimodal.effective_limits(), (16, 1));
    assert_eq!(multimodal.clamp_image_count(120), 99);

    let mut local_ai = openhuman_core::config::schema::LocalAiConfig {
        runtime_enabled: false,
        usage: openhuman_core::config::schema::LocalAiUsage {
            embeddings: true,
            heartbeat: true,
            subconscious: true,
        },
        ..Default::default()
    };
    assert!(!local_ai.is_active());
    local_ai.runtime_enabled = true;
    assert!(local_ai.is_active());
    let routing = openhuman_core::config::Config {
        local_ai,
        embeddings_provider: Some("ollama:nomic-embed-text".into()),
        ..Default::default()
    };
    assert!(routing.workload_uses_local("embeddings"));
    assert!(!routing.workload_uses_local("heartbeat"));
    assert!(!routing.workload_uses_local("subconscious"));

    let mut search = openhuman_core::config::schema::SearchConfig {
        brave: openhuman_core::config::schema::SearchEngineCredentials {
            base_url: None,
            api_key: Some(" brave-key ".into()),
        },
        ..Default::default()
    };
    assert_eq!(
        search.brave.key(),
        Some("brave-key"),
        "search credential keys should be trimmed at read time"
    );
    search
        .apply_legacy_engine(" Brave ")
        .expect("legacy engine applies");
    assert_eq!(
        search.roles.get("search"),
        Some(&vec!["brave".to_string(), "exa".to_string()])
    );
    assert!(search.apply_legacy_engine("bing").is_err());

    let integration = openhuman_core::config::schema::IntegrationToggle {
        enabled: true,
        mode: "byo".into(),
        api_key: Some("   ".into()),
    };
    assert!(!integration.is_active());
    let managed_integration = openhuman_core::config::schema::IntegrationToggle {
        enabled: true,
        mode: "managed".into(),
        api_key: None,
    };
    assert!(managed_integration.is_active());

    let mcp_default = openhuman_core::config::schema::McpServerConfig::default();
    assert!(mcp_default.enabled);
    assert_eq!(mcp_default.timeout_secs, 30);
    assert!(matches!(
        mcp_default.auth,
        openhuman_core::config::schema::McpAuthConfig::None
    ));
    let mcp_with_auth: openhuman_core::config::schema::McpServerConfig =
        serde_json::from_value(json!({
            "name": "worker-a-mcp",
            "endpoint": "https://mcp.example.test",
            "auth": {
                "kind": "header",
                "name": "x-api-key",
                "value": "secret"
            }
        }))
        .expect("mcp server auth config");
    assert!(matches!(
        mcp_with_auth.auth,
        openhuman_core::config::schema::McpAuthConfig::Header { .. }
    ));
    for auth in [
        json!({ "kind": "bearer_token", "token": "bearer" }),
        json!({ "kind": "basic", "username": "u", "password": "p" }),
        json!({ "kind": "query_param", "name": "api_key", "value": "secret" }),
    ] {
        let _: openhuman_core::config::schema::McpAuthConfig =
            serde_json::from_value(auth).expect("mcp auth variant should deserialize");
    }
}

#[test]
fn config_active_user_and_daemon_public_helpers_cover_path_branches() {
    let tmp = tempdir().expect("tempdir");
    let root = tmp.path().join(".openhuman");

    assert_eq!(read_active_user_id(&root), None);
    std::fs::create_dir_all(&root).expect("create root");
    std::fs::write(root.join("active_user.toml"), "user_id = \"   \"\n")
        .expect("write blank active user");
    assert_eq!(read_active_user_id(&root), None);
    std::fs::write(root.join("active_user.toml"), "not = [toml\n")
        .expect("write malformed active user");
    assert_eq!(read_active_user_id(&root), None);

    write_active_user_id(&root, "user-77").expect("write active user");
    assert_eq!(read_active_user_id(&root).as_deref(), Some("user-77"));
    assert_eq!(
        user_openhuman_dir(&root, "user-77"),
        root.join("users").join("user-77")
    );
    assert_eq!(pre_login_user_dir(&root), root.join("users").join("local"));

    clear_active_user(&root).expect("clear active user");
    clear_active_user(&root).expect("clearing missing active user is idempotent");
    assert_eq!(read_active_user_id(&root), None);

    let daemon = DaemonConfig::from_app_data_dir(tmp.path());
    assert_eq!(daemon.data_dir, tmp.path().join("openhuman"));
    assert_eq!(
        daemon.workspace_dir,
        tmp.path().join("openhuman").join("workspace")
    );
    assert!(daemon.security.audit.enabled);
}

#[test]
fn config_proxy_public_paths_normalize_validate_and_apply_scope() {
    let _lock = env_lock();
    let _http = EnvVarGuard::unset("HTTP_PROXY");
    let _https = EnvVarGuard::unset("HTTPS_PROXY");
    let _all = EnvVarGuard::unset("ALL_PROXY");
    let _no = EnvVarGuard::unset("NO_PROXY");
    let _http_lower = EnvVarGuard::unset("http_proxy");
    let _https_lower = EnvVarGuard::unset("https_proxy");
    let _all_lower = EnvVarGuard::unset("all_proxy");
    let _no_lower = EnvVarGuard::unset("no_proxy");

    assert!(ProxyConfig::supported_service_keys().contains(&"memory.embeddings"));
    assert!(ProxyConfig::supported_service_selectors().contains(&"tool.*"));

    let services = ProxyConfig {
        enabled: true,
        http_proxy: Some(" http://proxy.example:8080 ".into()),
        https_proxy: Some("https://secure-proxy.example".into()),
        all_proxy: None,
        no_proxy: vec![" localhost, 127.0.0.1 ".into(), "example.test".into()],
        scope: ProxyScope::Services,
        services: vec![
            " Tool.* ".into(),
            "tool.browser".into(),
            "memory.embeddings".into(),
        ],
    };
    services.validate().expect("valid services proxy");
    assert_eq!(
        services.normalized_services(),
        vec!["memory.embeddings", "tool.*", "tool.browser"]
    );
    assert_eq!(
        services.normalized_no_proxy(),
        vec!["127.0.0.1", "example.test", "localhost"]
    );
    assert!(services.should_apply_to_service("tool.http_request"));
    assert!(services.should_apply_to_service("memory.embeddings"));
    assert!(!services.should_apply_to_service("provider.openai"));
    assert!(!services.should_apply_to_service("   "));
    let _client = services
        .apply_to_reqwest_builder(reqwest::Client::builder(), "tool.browser")
        .build()
        .expect("proxied client builds");

    let env_scope = ProxyConfig {
        enabled: true,
        scope: ProxyScope::Environment,
        all_proxy: Some("socks5h://proxy.example:1080".into()),
        ..ProxyConfig::default()
    };
    env_scope.validate().expect("valid env proxy");
    assert!(!env_scope.should_apply_to_service("tool.browser"));
    env_scope.apply_to_process_env();
    assert_eq!(
        std::env::var("ALL_PROXY").as_deref(),
        Ok("socks5h://proxy.example:1080")
    );
    assert_eq!(
        std::env::var("all_proxy").as_deref(),
        Ok("socks5h://proxy.example:1080")
    );
    assert!(std::env::var("NO_PROXY").is_err());

    ProxyConfig::clear_process_env();
    assert!(std::env::var("ALL_PROXY").is_err());
    assert!(std::env::var("all_proxy").is_err());

    let openhuman_scope = ProxyConfig {
        enabled: true,
        scope: ProxyScope::OpenHuman,
        http_proxy: Some("https://proxy.example".into()),
        no_proxy: vec![" local.test ".into()],
        ..ProxyConfig::default()
    };
    assert!(openhuman_scope.has_any_proxy_url());
    assert!(openhuman_scope.should_apply_to_service("provider.openai"));
    assert_eq!(openhuman_scope.normalized_no_proxy(), vec!["local.test"]);
    openhuman_scope.apply_to_process_env();
    assert_eq!(
        std::env::var("HTTP_PROXY").as_deref(),
        Ok("https://proxy.example")
    );
    assert_eq!(std::env::var("NO_PROXY").as_deref(), Ok("local.test"));
    ProxyConfig::clear_process_env();

    for mut invalid in [
        ProxyConfig {
            enabled: true,
            http_proxy: Some("ftp://proxy.example".into()),
            ..ProxyConfig::default()
        },
        ProxyConfig {
            enabled: true,
            scope: ProxyScope::Services,
            services: vec![],
            http_proxy: Some("http://proxy.example".into()),
            ..ProxyConfig::default()
        },
        ProxyConfig {
            enabled: true,
            http_proxy: None,
            https_proxy: None,
            all_proxy: None,
            ..ProxyConfig::default()
        },
        ProxyConfig {
            enabled: false,
            services: vec!["unknown.service".into()],
            ..ProxyConfig::default()
        },
    ] {
        assert!(
            invalid.validate().is_err(),
            "invalid proxy config should fail: {invalid:?}"
        );
        invalid.enabled = false;
    }

    openhuman_core::config::set_runtime_proxy_config(services.clone());
    assert!(openhuman_core::config::runtime_proxy_config().should_apply_to_service("tool.browser"));
    let _cached = openhuman_core::config::build_runtime_proxy_client("tool.browser");
    let _cached_again = openhuman_core::config::build_runtime_proxy_client("tool.browser");
    let _timeout_client =
        openhuman_core::config::build_runtime_proxy_client_with_timeouts("memory.embeddings", 1, 1);
    let _builder = openhuman_core::config::apply_runtime_proxy_to_builder(
        reqwest::Client::builder(),
        "tool.http_request",
    );
    openhuman_core::config::set_runtime_proxy_config(ProxyConfig::default());
}

#[tokio::test]
async fn config_loaders_resolve_user_workspace_markers_and_ignore_workspace_when_scoped() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let root = home.join(".openhuman");
    let user_dir = root.join("users").join("user-42");
    let explicit_config_dir = tmp.path().join("explicit");
    let explicit_workspace = tmp.path().join("explicit-workspace");
    let env_workspace = tmp.path().join("env-workspace");
    let legacy_parent = tmp.path().join("legacy-parent");
    let legacy_config_dir = legacy_parent.join(".openhuman");
    let legacy_workspace = legacy_parent.join("workspace");

    let _guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset("OPENHUMAN_MODEL"),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
        EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];

    std::fs::create_dir_all(&root).expect("create root config dir");
    write_active_user_id(&root, "user-42").expect("write active user marker");
    write_min_config(&user_dir);

    let active_user_config = Config::load_or_init()
        .await
        .expect("load active user config");
    assert_eq!(active_user_config.config_path, user_dir.join("config.toml"));
    assert_eq!(active_user_config.workspace_dir, user_dir.join("workspace"));

    {
        write_min_config(&env_workspace);
        let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &env_workspace);
        let env_config = Config::load_or_init()
            .await
            .expect("load env workspace config");
        assert_eq!(env_config.config_path, env_workspace.join("config.toml"));
        assert_eq!(env_config.workspace_dir, env_workspace.join("workspace"));
    }

    {
        write_min_config(&legacy_config_dir);
        std::fs::create_dir_all(&legacy_workspace).expect("create legacy workspace");
        let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &legacy_workspace);
        let legacy_config = Config::load_or_init()
            .await
            .expect("load legacy workspace config");
        assert_eq!(
            legacy_config.config_path,
            legacy_config_dir.join("config.toml")
        );
        assert_eq!(legacy_config.workspace_dir, legacy_workspace);
    }

    clear_active_user(&root).expect("clear active user marker");
    std::fs::write(root.join("active_workspace.toml"), "config_dir = [\n")
        .expect("write malformed active workspace marker");
    let malformed_marker_config = Config::load_or_init()
        .await
        .expect("malformed active workspace marker should be ignored");
    assert_eq!(
        malformed_marker_config.config_path,
        root.join("users").join("local").join("config.toml")
    );

    std::fs::remove_file(root.join("active_workspace.toml"))
        .expect("remove malformed active workspace marker");
    std::fs::create_dir(root.join("active_workspace.toml"))
        .expect("create unreadable active workspace marker");
    let unreadable_marker_config = Config::load_or_init()
        .await
        .expect("unreadable active workspace marker should be ignored");
    assert_eq!(
        unreadable_marker_config.config_path,
        root.join("users").join("local").join("config.toml")
    );
    std::fs::remove_dir(root.join("active_workspace.toml"))
        .expect("remove unreadable active workspace marker");

    let active_marker_dir = root.join("relative-active");
    write_min_config(&active_marker_dir);
    std::fs::write(
        root.join("active_workspace.toml"),
        "config_dir = \"relative-active\"\n",
    )
    .expect("write active workspace marker");
    let marker_config = Config::load_or_init()
        .await
        .expect("load active workspace marker config");
    assert_eq!(
        marker_config.config_path,
        active_marker_dir.join("config.toml")
    );
    assert_eq!(
        marker_config.workspace_dir,
        active_marker_dir.join("workspace")
    );

    write_min_config(&explicit_config_dir);
    let _workspace_guard = EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &env_workspace);
    let _model_guard = EnvVarGuard::set("OPENHUMAN_MODEL", " scoped-env-model ");
    let explicit = Config::load_from_config_path(
        &explicit_config_dir.join("config.toml"),
        &explicit_workspace,
    )
    .await
    .expect("load explicit config path");
    assert_eq!(explicit.workspace_dir, explicit_workspace);
    assert_eq!(explicit.default_model.as_deref(), Some("scoped-env-model"));
}

#[tokio::test]
async fn config_default_path_loader_ignores_workspace_override_and_projects_dir_trims() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let root = home.join(".openhuman");
    let user_dir = root.join("users").join("default-loader-user");
    let workspace_override = tmp.path().join("workspace-override");
    let _guards = [
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
        EnvVarGuard::set_to_path("OPENHUMAN_WORKSPACE", &workspace_override),
        EnvVarGuard::set("OPENHUMAN_MODEL", " default-loader-model "),
    ];

    let missing = Config::load_from_default_paths()
        .await
        .expect("load default paths without an existing config");
    assert_eq!(
        missing.config_path,
        root.join("users").join("local").join("config.toml")
    );
    assert_eq!(missing.workspace_dir, workspace_override.join("workspace"));
    assert_eq!(
        missing.default_model.as_deref(),
        Some("default-loader-model")
    );

    write_active_user_id(&root, "default-loader-user").expect("write active user");
    write_min_config(&user_dir);
    write_min_config(&workspace_override);
    let loaded = Config::load_from_default_paths()
        .await
        .expect("load default active-user config while workspace override is set");
    assert_eq!(loaded.config_path, user_dir.join("config.toml"));
    assert_eq!(loaded.workspace_dir, workspace_override.join("workspace"));
    assert_eq!(
        loaded.default_model.as_deref(),
        Some("default-loader-model")
    );

    let custom_projects = tmp.path().join("OpenHuman Projects");
    {
        let _projects_guard = EnvVarGuard::set_to_path("OPENHUMAN_PROJECTS_DIR", &custom_projects);
        assert_eq!(default_projects_dir(), custom_projects);
    }
    let _blank_projects_guard = EnvVarGuard::set("OPENHUMAN_PROJECTS_DIR", "   ");
    assert_eq!(
        default_projects_dir(),
        home.join("OpenHuman").join("projects")
    );
}

#[tokio::test]
async fn config_env_overlay_public_loader_applies_runtime_and_tool_overrides() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let config_dir = tmp.path().join("config");
    let workspace_dir = tmp.path().join("workspace");
    write_min_config(&config_dir);

    let _guards = vec![
        EnvVarGuard::set_to_path("HOME", tmp.path()),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::set("OPENHUMAN_MODEL", " env-model "),
        EnvVarGuard::set("OPENHUMAN_TEMPERATURE", "1.25"),
        EnvVarGuard::set("OPENHUMAN_MAX_ACTIONS_PER_HOUR", "17"),
        EnvVarGuard::set("OPENHUMAN_OUTPUT_LANGUAGE", " ja "),
        EnvVarGuard::set("OPENHUMAN_REASONING_ENABLED", "yes"),
        EnvVarGuard::set("OPENHUMAN_SELTZ_API_KEY", "seltz-key"),
        EnvVarGuard::set("OPENHUMAN_SELTZ_API_URL", "https://seltz.example/v1"),
        EnvVarGuard::set("OPENHUMAN_SELTZ_MAX_RESULTS", "13"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_ENABLED", "on"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_BASE_URL", "https://searx.example"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_MAX_RESULTS", "31"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_DEFAULT_LANGUAGE", "de"),
        EnvVarGuard::set("OPENHUMAN_SEARXNG_TIMEOUT_SECS", "9"),
        EnvVarGuard::set("OPENHUMAN_SEARCH_ENGINE", "brave"),
        EnvVarGuard::set("OPENHUMAN_PARALLEL_API_KEY", "parallel-key"),
        EnvVarGuard::set("OPENHUMAN_BRAVE_API_KEY", "brave-key"),
        EnvVarGuard::set("OPENHUMAN_QUERIT_API_KEY", "querit-key"),
        EnvVarGuard::set("OPENHUMAN_EXA_API_KEY", "exa-key"),
        EnvVarGuard::set("OPENHUMAN_SEARCH_MAX_RESULTS", "11"),
        EnvVarGuard::set("OPENHUMAN_SEARCH_TIMEOUT_SECS", "8"),
        EnvVarGuard::set("OPENHUMAN_WEB_SEARCH_ENABLED", "0"),
        EnvVarGuard::set("OPENHUMAN_WEB_SEARCH_MAX_RESULTS", "7"),
        EnvVarGuard::set("OPENHUMAN_WEB_SEARCH_TIMEOUT_SECS", "6"),
        EnvVarGuard::set("OPENHUMAN_PROXY_ENABLED", "true"),
        EnvVarGuard::set("OPENHUMAN_HTTP_PROXY", " http://proxy.example:8080 "),
        EnvVarGuard::set("OPENHUMAN_NO_PROXY", " localhost,example.test "),
        EnvVarGuard::set("OPENHUMAN_PROXY_SCOPE", "services"),
        EnvVarGuard::set("OPENHUMAN_PROXY_SERVICES", "tool.browser,memory.embeddings"),
        EnvVarGuard::set("OPENHUMAN_NODE_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_NODE_VERSION", "v24.0.0"),
        EnvVarGuard::set("OPENHUMAN_NODE_CACHE_DIR", "/tmp/openhuman-node-cache"),
        EnvVarGuard::set("OPENHUMAN_NODE_PREFER_SYSTEM", "false"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_MINIMUM_VERSION", "3.13.0"),
        EnvVarGuard::set(
            "OPENHUMAN_RUNTIME_PYTHON_CACHE_DIR",
            "/tmp/openhuman-python-cache",
        ),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_MANAGED_RELEASE_TAG", "20260401"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_PREFER_SYSTEM", "true"),
        EnvVarGuard::set("OPENHUMAN_RUNTIME_PYTHON_PREFERRED_COMMAND", "python3.13"),
        EnvVarGuard::set("OPENHUMAN_CORE_SENTRY_DSN", "https://dsn.example/1"),
        EnvVarGuard::set("OPENHUMAN_ANALYTICS_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_INTERVAL_MINUTES", "1440"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_RESTART_STRATEGY", "supervisor"),
        EnvVarGuard::set("OPENHUMAN_AUTO_UPDATE_RPC_MUTATIONS_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_MICROCOMPACT_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_AUTOCOMPACT_ENABLED", "false"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_TOOL_RESULT_BUDGET_BYTES", "12345"),
        EnvVarGuard::set("OPENHUMAN_CONTEXT_SUMMARIZER_MODEL", "summary-env"),
    ];

    let config = Config::load_from_config_path(&config_dir.join("config.toml"), &workspace_dir)
        .await
        .expect("load config with env overlay");

    assert_eq!(config.default_model.as_deref(), Some("env-model"));
    assert_eq!(config.default_temperature, 1.25);
    assert_eq!(config.autonomy.max_actions_per_hour, 17);
    assert_eq!(config.output_language.as_deref(), Some("ja"));
    assert_eq!(config.runtime.reasoning_enabled, Some(true));
    assert!(config.seltz.enabled);
    assert_eq!(config.seltz.api_key.as_deref(), Some("seltz-key"));
    assert_eq!(config.seltz.max_results, 13);
    assert!(config.searxng.enabled);
    assert_eq!(config.searxng.base_url, "https://searx.example");
    assert_eq!(config.searxng.max_results, 31);
    assert_eq!(
        config
            .search
            .roles
            .get("search")
            .and_then(|order| order.first()),
        Some(&"brave".to_string())
    );
    assert!(config.search.parallel.has_key());
    assert!(config.search.brave.has_key());
    assert!(config.search.querit.has_key());
    assert!(config.search.exa.has_key());
    assert_eq!(config.search.max_results, 11);
    assert_eq!(config.web_search.max_results, 7);
    assert!(config.proxy.enabled);
    assert_eq!(config.proxy.scope, ProxyScope::Services);
    assert!(config.proxy.should_apply_to_service("tool.browser"));
    // Deprecated runtime environment settings must not resurrect removed fields.
    let serialized = serde_json::to_value(&config).expect("serialize config");
    assert!(serialized.get("node").is_none());
    assert!(serialized.get("runtime_python").is_none());
    assert!(!config.observability.analytics_enabled);
    assert_eq!(
        config.observability.sentry_dsn.as_deref(),
        Some("https://dsn.example/1")
    );
    assert!(!config.update.enabled);
    assert_eq!(config.update.interval_minutes, 1440);
    assert_eq!(
        config.update.restart_strategy,
        UpdateRestartStrategy::Supervisor
    );
    assert!(!config.update.rpc_mutations_enabled);
    assert!(!config.context.enabled);
    assert!(!config.context.microcompact_enabled);
    assert!(!config.context.autocompact_enabled);
    assert_eq!(config.context.tool_result_budget_bytes, 12345);
    assert_eq!(
        config.context.summarizer_model.as_deref(),
        Some("summary-env")
    );
}

#[tokio::test]
async fn config_save_and_load_encrypts_channel_secret_fields() {
    let _lock = env_lock_async().await;
    let _keyring_guard = EnvVarGuard::set("OPENHUMAN_KEYRING_BACKEND", "file");
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let _guards = vec![
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
        EnvVarGuard::unset("OPENHUMAN_TAVILY_API_KEY"),
        EnvVarGuard::unset("TAVILY_API_KEY"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_STRICT", "false"),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_ENDPOINT", ""),
        EnvVarGuard::set("OPENHUMAN_MEMORY_EMBED_MODEL", ""),
    ];
    let config_path = home
        .join(".openhuman")
        .join("users")
        .join("local")
        .join("config.toml");
    let workspace_dir = config_path
        .parent()
        .expect("config parent")
        .join("workspace");

    let mut config = Config {
        config_path: config_path.clone(),
        workspace_dir: workspace_dir.clone(),
        ..Default::default()
    };
    config.secrets.encrypt = true;
    config.api_key = Some("api-secret".into());
    config.search.brave.api_key = Some("brave-secret".into());
    config.search.querit.api_key = Some("querit-secret".into());
    config.search.exa.api_key = Some("exa-secret".into());
    config.search.tavily.api_key = Some("tavily-secret".into());
    config.channels_config.telegram = Some(TelegramConfig {
        bot_token: "telegram-secret".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    config.channels_config.discord = Some(DiscordConfig {
        bot_token: "discord-secret".into(),
        guild_id: Some("guild".into()),
        channel_id: Some("channel".into()),
        allowed_users: vec![],
        listen_to_bots: false,
        mention_only: false,
    });
    config.channels_config.slack = Some(SlackConfig {
        bot_token: "slack-bot-secret".into(),
        app_token: Some("slack-app-secret".into()),
        channel_id: Some("C123".into()),
        allowed_users: vec![],
    });
    config.channels_config.whatsapp = Some(WhatsAppConfig {
        access_token: Some("whatsapp-access-secret".into()),
        phone_number_id: Some("phone".into()),
        verify_token: Some("whatsapp-verify-secret".into()),
        app_secret: Some("whatsapp-app-secret".into()),
        session_path: None,
        pair_phone: None,
        pair_code: None,
        allowed_numbers: vec![],
    });
    config.channels_config.irc = Some(IrcConfig {
        server: "irc.example.test".into(),
        port: 6697,
        nickname: "worker".into(),
        username: Some("worker".into()),
        channels: vec!["#openhuman".into()],
        allowed_users: vec![],
        server_password: Some("irc-server-secret".into()),
        nickserv_password: Some("irc-nickserv-secret".into()),
        sasl_password: Some("irc-sasl-secret".into()),
        verify_tls: Some(true),
    });
    config.channels_config.lark = Some(LarkConfig {
        app_id: "lark-app".into(),
        app_secret: "lark-app-secret".into(),
        encrypt_key: Some("lark-encrypt-secret".into()),
        verification_token: Some("lark-verify-secret".into()),
        allowed_users: vec![],
        use_feishu: false,
        receive_mode: Default::default(),
        port: None,
    });
    config.channels_config.dingtalk = Some(DingTalkConfig {
        client_id: "dingtalk-client".into(),
        client_secret: "dingtalk-secret".into(),
        allowed_users: vec![],
    });
    config.channels_config.qq = Some(QQConfig {
        app_id: "qq-app".into(),
        app_secret: "qq-secret".into(),
        allowed_users: vec![],
    });

    config.save().await.expect("save encrypted config");
    let raw = std::fs::read_to_string(&config_path).expect("read saved encrypted config");
    for secret in [
        "api-secret",
        "exa-secret",
        "tavily-secret",
        "telegram-secret",
        "discord-secret",
        "slack-bot-secret",
        "slack-app-secret",
        "whatsapp-access-secret",
        "irc-server-secret",
        "lark-app-secret",
        "dingtalk-secret",
        "qq-secret",
    ] {
        assert!(
            !raw.contains(secret),
            "saved encrypted config should not contain plaintext {secret}: {raw}"
        );
    }

    let loaded = Config::load_or_init()
        .await
        .expect("load encrypted config from default path");
    assert_eq!(loaded.config_path, config_path);
    assert_eq!(loaded.api_key.as_deref(), Some("api-secret"));
    assert_eq!(loaded.search.exa.api_key.as_deref(), Some("exa-secret"));
    assert_eq!(
        loaded.search.tavily.api_key.as_deref(),
        Some("tavily-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .telegram
            .as_ref()
            .map(|telegram| telegram.bot_token.as_str()),
        Some("telegram-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .slack
            .as_ref()
            .and_then(|slack| slack.app_token.as_deref()),
        Some("slack-app-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .lark
            .as_ref()
            .map(|lark| lark.app_secret.as_str()),
        Some("lark-app-secret")
    );
    assert_eq!(
        loaded
            .channels_config
            .qq
            .as_ref()
            .map(|qq| qq.app_secret.as_str()),
        Some("qq-secret")
    );
}

#[tokio::test]
async fn worker_a_controller_schemas_are_fully_exposed() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let schema = schema(&harness.rpc_base).await;

    for (namespace, expected) in [
        (
            "config",
            vec![
                "openhuman.config_agent_server_status",
                "openhuman.config_get",
                "openhuman.config_get_agent_paths",
                "openhuman.config_get_agent_settings",
                "openhuman.config_get_analytics_settings",
                "openhuman.config_get_autonomy_settings",
                "openhuman.config_get_client_config",
                "openhuman.config_get_composio_trigger_settings",
                "openhuman.config_get_dashboard_settings",
                "openhuman.config_get_data_paths",
                "openhuman.config_get_onboarding_completed",
                "openhuman.config_get_privacy_mode",
                "openhuman.config_get_runtime_flags",
                "openhuman.config_get_sandbox_settings",
                "openhuman.config_get_search_settings",
                "openhuman.config_get_user_timezone",
                "openhuman.config_reset_local_data",
                "openhuman.config_resolve_api_url",
                "openhuman.config_set_browser_allow_all",
                "openhuman.config_set_onboarding_completed",
                "openhuman.config_set_privacy_mode",
                "openhuman.config_update_agent_paths",
                "openhuman.config_update_agent_settings",
                "openhuman.config_update_analytics_settings",
                "openhuman.config_update_autonomy_settings",
                "openhuman.config_update_browser_settings",
                "openhuman.config_update_composio_trigger_settings",
                "openhuman.config_update_computer_settings",
                "openhuman.config_update_local_ai_settings",
                "openhuman.config_update_memory_settings",
                "openhuman.config_update_model_settings",
                "openhuman.config_update_runtime_settings",
                "openhuman.config_update_sandbox_settings",
                "openhuman.config_update_search_settings",
                "openhuman.config_update_user_timezone",
                "openhuman.config_workspace_onboarding_flag_exists",
                "openhuman.config_workspace_onboarding_flag_set",
            ],
        ),
        (
            "auth",
            vec![
                "openhuman.auth_clear_credential",
                "openhuman.auth_create_channel_link_token",
                "openhuman.auth_get_session_token",
                "openhuman.auth_get_state",
                "openhuman.auth_list_provider_credentials",
                "openhuman.auth_oauth_connect",
                "openhuman.auth_oauth_fetch_client_key",
                "openhuman.auth_oauth_fetch_integration_tokens",
                "openhuman.auth_oauth_list_integrations",
                "openhuman.auth_oauth_revoke_integration",
                "openhuman.auth_remove_provider_credentials",
                "openhuman.auth_set_credential",
                "openhuman.auth_store_provider_credentials",
            ],
        ),
        (
            "app_state",
            vec![
                "openhuman.app_state_snapshot",
                "openhuman.app_state_update_local_state",
            ],
        ),
        ("connectivity", vec!["openhuman.connectivity_diag"]),
        (
            "memory",
            vec![
                "openhuman.memory_agents_list",
                "openhuman.memory_brain_forget",
                "openhuman.memory_brain_ingest",
                "openhuman.memory_brain_search",
                "openhuman.memory_brain_sources",
                "openhuman.memory_conversations_backfill_start",
                "openhuman.memory_conversations_backfill_status",
                "openhuman.memory_engine_get",
                "openhuman.memory_engine_set",
                "openhuman.memory_engines_list",
                "openhuman.memory_erase_all",
                "openhuman.memory_explore",
                "openhuman.memory_fetch",
                "openhuman.memory_forget",
                "openhuman.memory_import_retry_failed",
                "openhuman.memory_import_scan",
                "openhuman.memory_import_start",
                "openhuman.memory_import_status",
                "openhuman.memory_items_get",
                "openhuman.memory_items_list",
                "openhuman.memory_jobs_list",
                "openhuman.memory_jobs_run",
                "openhuman.memory_learn",
                "openhuman.memory_migration_retry",
                "openhuman.memory_migration_scan",
                "openhuman.memory_migration_start",
                "openhuman.memory_migration_status",
                "openhuman.memory_pack_preview",
                "openhuman.memory_policy_get",
                "openhuman.memory_policy_set",
                "openhuman.memory_recall",
                "openhuman.memory_sources_add",
                "openhuman.memory_sources_list",
                "openhuman.memory_sources_remove",
                "openhuman.memory_sources_sync",
            ],
        ),
    ] {
        assert_eq!(
            schema_method_names(&schema, namespace),
            expected,
            "schema catalog mismatch for namespace {namespace}"
        );
    }

    // Inference/agent/tools/approval methods must also be advertised.
    let advertised: Vec<String> = schema
        .get("methods")
        .and_then(Value::as_array)
        .expect("schema methods array")
        .iter()
        .filter_map(|method| method.get("method").and_then(Value::as_str))
        .map(str::to_string)
        .collect();
    for expected in [
        "openhuman.inference_status",
        "openhuman.inference_get_client_config",
        "openhuman.inference_update_model_settings",
        "openhuman.inference_update_local_settings",
        "openhuman.inference_list_models",
        "openhuman.inference_diagnostics",
        "openhuman.inference_openai_oauth_start",
        "openhuman.inference_openai_oauth_complete",
        "openhuman.inference_openai_oauth_status",
        "openhuman.inference_openai_oauth_disconnect",
        "openhuman.inference_summarize",
        "openhuman.inference_prompt",
        "openhuman.inference_vision_prompt",
        "openhuman.inference_test_provider_model",
        "openhuman.inference_analyze_sentiment",
        "openhuman.agent_chat",
        "openhuman.agent_chat_simple",
        "openhuman.agent_server_status",
        "openhuman.agent_list_definitions",
        "openhuman.agent_get_definition",
        "openhuman.agent_reload_definitions",
        "openhuman.agent_triage_evaluate",
        "openhuman.tools_composio_execute",
        "openhuman.tools_web_search",
        "openhuman.tools_web_answer",
        "openhuman.tools_web_contents",
        "openhuman.tools_searxng_search",
        "openhuman.tool_registry_list",
        "openhuman.tool_registry_get",
        "openhuman.tool_registry_diagnostics",
        "openhuman.approval_list_pending",
        "openhuman.approval_list_recent_decisions",
        "openhuman.approval_decide",
    ] {
        assert!(
            advertised.iter().any(|method| method == expected),
            "schema catalog must expose {expected}"
        );
    }

    let unknown_app_state = app_state_schemas("missing");
    assert_eq!(unknown_app_state.namespace, "app_state");
    assert_eq!(unknown_app_state.function, "unknown");
    assert_eq!(unknown_app_state.outputs[0].name, "error");
    assert!(unknown_app_state.description.contains("Unknown app_state"));

    harness.join.abort();
}

#[tokio::test]
async fn config_controller_mutations_round_trip_over_json_rpc() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let initial = rpc(&harness.rpc_base, 10_001, "openhuman.config_get", json!({})).await;
    assert!(
        payload(&initial, "config_get")
            .get("workspace_dir")
            .and_then(Value::as_str)
            .is_some(),
        "config_get should expose resolved paths: {initial}"
    );

    let model = rpc(
        &harness.rpc_base,
        10_002,
        "openhuman.config_update_model_settings",
        json!({
            "api_url": "http://127.0.0.1:9",
            "inference_url": "http://127.0.0.1:19999/v1",
            "api_key": "worker-a-secret",
            "default_model": "worker-a-updated",
            "default_temperature": 0.4,
            "model_routes": [{ "hint": "reasoning", "model": "route-model" }],
            "cloud_providers": [{
                "id": "provider-a",
                "slug": "worker-a-cloud",
                "label": "Worker A Cloud",
                "endpoint": "http://127.0.0.1:19999/v1",
                "auth_style": "bearer"
            }],
            "primary_cloud": "provider-a",
            "chat_provider": "worker-a-cloud:chat",
            "reasoning_provider": "worker-a-cloud:reason",
            "agentic_provider": "worker-a-cloud:agent",
            "coding_provider": "worker-a-cloud:code",
            "memory_provider": "worker-a-cloud:memory",
            "embeddings_provider": "worker-a-cloud:embeddings"
        }),
    )
    .await;
    ok(&model, "update_model_settings");

    let client = rpc(
        &harness.rpc_base,
        10_003,
        "openhuman.config_get_client_config",
        json!({}),
    )
    .await;
    let client_payload = payload(&client, "get_client_config");
    assert_eq!(
        client_payload.get("default_model").and_then(Value::as_str),
        Some("worker-a-updated")
    );
    assert_eq!(
        client_payload.get("api_key_set").and_then(Value::as_bool),
        Some(true),
        "client config should expose only API key presence: {client_payload}"
    );
    assert!(
        !client_payload.to_string().contains("worker-a-secret"),
        "client config must not echo local API keys: {client_payload}"
    );

    // The model registry is seeded + price/context-window-enriched from the
    // static pricing catalog on load (in-memory), so a fresh workspace surfaces
    // real numbers over RPC. Validates the full path: catalog → seed-on-load →
    // client_config_json → JSON-RPC. (The earlier update_model_settings call did
    // not include `model_registry`, so the seeded registry is left intact.)
    let registry = client_payload
        .get("model_registry")
        .and_then(Value::as_array)
        .expect("client config should expose model_registry");
    assert!(
        !registry.is_empty(),
        "model_registry should be auto-seeded from the pricing catalog: {client_payload}"
    );
    let opus = registry
        .iter()
        .find(|m| m.get("id").and_then(Value::as_str) == Some("claude-opus-4-8"))
        .unwrap_or_else(|| panic!("seeded registry should contain claude-opus-4-8: {registry:?}"));
    assert_eq!(
        opus.get("provider").and_then(Value::as_str),
        Some("anthropic")
    );
    assert_eq!(
        opus.get("cost_per_1m_input").and_then(Value::as_f64),
        Some(5.0),
        "input price should be pre-filled from the catalog: {opus:?}"
    );
    assert_eq!(
        opus.get("cost_per_1m_output").and_then(Value::as_f64),
        Some(25.0),
        "output price should be pre-filled from the catalog: {opus:?}"
    );
    assert_eq!(
        opus.get("context_window").and_then(Value::as_u64),
        Some(1_000_000),
        "context window should be pre-filled from the catalog: {opus:?}"
    );
    // Every seeded entry carries non-zero pricing + context window — the whole
    // point of the catalog pre-fill.
    for entry in registry {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or("<none>");
        assert!(
            entry
                .get("cost_per_1m_output")
                .and_then(Value::as_f64)
                .unwrap_or(0.0)
                > 0.0,
            "{id} missing output price: {entry:?}"
        );
        assert!(
            entry
                .get("context_window")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0,
            "{id} missing context window: {entry:?}"
        );
    }

    let memory = rpc(
        &harness.rpc_base,
        10_004,
        "openhuman.config_update_memory_settings",
        json!({
            "embedding_provider": "none",
            "embedding_model": "none",
            "embedding_dimensions": 0
        }),
    )
    .await;
    ok(&memory, "update_memory_settings");

    for (id, method, params) in [
        (
            10_005,
            "openhuman.config_update_runtime_settings",
            json!({ "kind": "local", "reasoning_enabled": true }),
        ),
        (
            10_006,
            "openhuman.config_update_browser_settings",
            json!({ "enabled": true }),
        ),
        (
            10_008,
            "openhuman.config_update_local_ai_settings",
            json!({
                "runtime_enabled": false,
                "opt_in_confirmed": false,
                "provider": "ollama",
                "base_url": "http://127.0.0.1:11434",
                "model_id": "llama3",
                "chat_model_id": "llama3",
                "usage_embeddings": false
            }),
        ),
        (
            10_010,
            "openhuman.config_update_composio_trigger_settings",
            json!({
                "triage_disabled": true,
                "triage_disabled_toolkits": ["gmail", "slack"]
            }),
        ),
        (
            10_011,
            "openhuman.config_update_autonomy_settings",
            json!({
                "level": "supervised",
                "workspace_only": true,
                "allowed_commands": ["git", "cargo"],
                "forbidden_paths": ["/tmp/forbidden-worker-a"],
                "trusted_roots": [{
                    "path": harness.home.display().to_string(),
                    "access": "read"
                }],
                "allow_tool_install": false,
                "max_actions_per_hour": 42,
                "auto_approve": ["memory.search"]
            }),
        ),
        (
            10_012,
            "openhuman.config_update_search_settings",
            json!({
                "engine": "managed",
                "max_results": 5,
                "timeout_secs": 12,
                "providers": {
                    "brave": {"enabled": true, "api_key": "brave-secret"},
                    "querit": {"enabled": true, "api_key": "querit-secret"}
                },
                "allowed_domains": ["example.com"],
                "allow_all": false
            }),
        ),
    ] {
        let response = rpc(&harness.rpc_base, id, method, params).await;
        ok(&response, method);
    }

    for (id, method) in [
        (10_101, "openhuman.config_resolve_api_url"),
        (10_102, "openhuman.config_get_runtime_flags"),
        (10_103, "openhuman.config_get_dashboard_settings"),
        (10_104, "openhuman.config_agent_server_status"),
        (10_105, "openhuman.config_get_data_paths"),
        (10_107, "openhuman.config_get_composio_trigger_settings"),
        (10_108, "openhuman.config_get_autonomy_settings"),
        (10_109, "openhuman.config_get_search_settings"),
    ] {
        let response = rpc(&harness.rpc_base, id, method, json!({})).await;
        ok(&response, method);
    }

    let allow_all = rpc(
        &harness.rpc_base,
        10_201,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": false }),
    )
    .await;
    ok(&allow_all, "set_browser_allow_all false");

    let exists_before = rpc(
        &harness.rpc_base,
        10_202,
        "openhuman.config_workspace_onboarding_flag_exists",
        json!({ "flag_name": ".worker-a-onboarding" }),
    )
    .await;
    assert_eq!(
        payload(&exists_before, "workspace_onboarding_flag_exists before").as_bool(),
        Some(false)
    );

    let set_flag = rpc(
        &harness.rpc_base,
        10_203,
        "openhuman.config_workspace_onboarding_flag_set",
        json!({ "flag_name": ".worker-a-onboarding", "value": true }),
    )
    .await;
    assert_eq!(
        payload(&set_flag, "workspace_onboarding_flag_set true").as_bool(),
        Some(true)
    );

    let clear_flag = rpc(
        &harness.rpc_base,
        10_204,
        "openhuman.config_workspace_onboarding_flag_set",
        json!({ "flag_name": ".worker-a-onboarding", "value": false }),
    )
    .await;
    assert_eq!(
        payload(&clear_flag, "workspace_onboarding_flag_set false").as_bool(),
        Some(false)
    );

    let reset = rpc(
        &harness.rpc_base,
        10_301,
        "openhuman.config_reset_local_data",
        json!({}),
    )
    .await;
    assert!(
        payload(&reset, "reset_local_data").is_object(),
        "reset should return a result payload: {reset}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn config_runtime_flags_settings_readbacks_and_validation_paths_are_exercised() {
    let _lock = env_lock_async().await;
    let harness = setup().await;

    let refused = rpc(
        &harness.rpc_base,
        11_001,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": true }),
    )
    .await;
    assert_error_contains(
        &refused,
        "set_browser_allow_all true without operator opt-in",
        "Refusing to enable OPENHUMAN_BROWSER_ALLOW_ALL",
    );

    std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL_RPC_ENABLE", "1");
    let enabled = rpc(
        &harness.rpc_base,
        11_002,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": true }),
    )
    .await;
    assert_eq!(
        payload(&enabled, "set_browser_allow_all true")
            .get("browser_allow_all")
            .and_then(Value::as_bool),
        Some(true)
    );

    let disabled = rpc(
        &harness.rpc_base,
        11_003,
        "openhuman.config_set_browser_allow_all",
        json!({ "enabled": false }),
    )
    .await;
    assert_eq!(
        payload(&disabled, "set_browser_allow_all false")
            .get("browser_allow_all")
            .and_then(Value::as_bool),
        Some(false)
    );

    ok(
        &rpc(
            &harness.rpc_base,
            11_004,
            "openhuman.config_update_analytics_settings",
            json!({ "enabled": false }),
        )
        .await,
        "update_analytics_settings false",
    );
    let analytics = rpc(
        &harness.rpc_base,
        11_005,
        "openhuman.config_get_analytics_settings",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&analytics, "get_analytics_settings")
            .get("enabled")
            .and_then(Value::as_bool),
        Some(false)
    );

    let onboarding_before = rpc(
        &harness.rpc_base,
        11_008,
        "openhuman.config_get_onboarding_completed",
        json!({}),
    )
    .await;
    assert_eq!(
        payload(&onboarding_before, "get_onboarding_completed before").as_bool(),
        Some(false)
    );
    for (id, value) in [(11_009, true), (11_010, false)] {
        let updated = rpc(
            &harness.rpc_base,
            id,
            "openhuman.config_set_onboarding_completed",
            json!({ "value": value }),
        )
        .await;
        assert_eq!(
            payload(&updated, "set_onboarding_completed").as_bool(),
            Some(value)
        );
    }

    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_015,
            "openhuman.config_update_search_settings",
            json!({ "engine": "bing" }),
        )
        .await,
        "update_search_settings invalid engine",
        "unknown search engine",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_016,
            "openhuman.config_update_search_settings",
            json!({ "max_results": 0 }),
        )
        .await,
        "update_search_settings invalid max_results",
        "max_results must be between",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_024,
            "openhuman.config_update_search_settings",
            json!({ "timeout_secs": 0 }),
        )
        .await,
        "update_search_settings invalid timeout_secs",
        "timeout_secs must be between",
    );
    let valid_search = rpc(
        &harness.rpc_base,
        11_025,
        "openhuman.config_update_search_settings",
        json!({
            "engine": " brave ",
            "max_results": 12,
            "timeout_secs": 42,
            "providers": {
                "brave": {"api_key": " brave-rpc-key "},
                "querit": {"enabled": true, "api_key": " querit-rpc-key "},
                "exa": {"api_key": " exa-rpc-key "},
                "tavily": {"enabled": true, "api_key": " tavily-rpc-key "}
            },
            "allowed_domains": [" example.com ", "", "example.com", "docs.example.com"],
            "allow_all": false
        }),
    )
    .await;
    let valid_search_payload = payload(&valid_search, "update_search_settings valid");
    assert_eq!(valid_search_payload.get("max_results"), Some(&json!(12)));
    assert_eq!(valid_search_payload.get("timeout_secs"), Some(&json!(42)));
    let search_readback = rpc(
        &harness.rpc_base,
        11_026,
        "openhuman.config_get_search_settings",
        json!({}),
    )
    .await;
    let search_payload = payload(&search_readback, "get_search_settings after valid update");
    let provider = |view: &Value, id: &str| -> Value {
        view.get("providers")
            .and_then(Value::as_array)
            .and_then(|providers| providers.iter().find(|p| p["id"] == id).cloned())
            .unwrap_or_else(|| panic!("provider {id} listed: {view}"))
    };
    for id in ["brave", "querit", "exa", "tavily"] {
        assert_eq!(
            provider(search_payload, id)["key_configured"],
            json!(true),
            "{id} key should be stored"
        );
    }
    assert_eq!(
        search_payload.pointer("/effective_roles/search/0"),
        Some(&json!("brave")),
        "the legacy engine selection leads the search role: {search_payload}"
    );
    assert!(!search_payload.to_string().contains("rpc-key"));
    assert_eq!(
        search_payload.get("allow_all").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        search_payload.get("allowed_domains"),
        Some(&json!(["docs.example.com", "example.com"]))
    );
    let select_tavily = rpc(
        &harness.rpc_base,
        11_126,
        "openhuman.config_update_search_settings",
        json!({ "roles": {"search": ["tavily", "brave"]} }),
    )
    .await;
    let select_tavily_payload = payload(&select_tavily, "put Tavily first for search");
    assert_eq!(
        select_tavily_payload.pointer("/effective_roles/search/0"),
        Some(&json!("tavily"))
    );
    assert_eq!(
        select_tavily_payload.pointer("/effective_roles/contents/0"),
        Some(&json!("tavily")),
        "Tavily also serves contents without a session: {select_tavily_payload}"
    );
    let allow_all_search = rpc(
        &harness.rpc_base,
        11_027,
        "openhuman.config_update_search_settings",
        json!({
            "providers": {
                "brave": {"api_key": " "},
                "querit": {"api_key": " "},
                "exa": {"api_key": " "},
                "tavily": {"api_key": " "}
            },
            "allow_all": true
        }),
    )
    .await;
    let allow_all_payload = payload(&allow_all_search, "update_search_settings allow_all");
    assert_eq!(
        allow_all_payload.get("allowed_domains"),
        Some(&json!(["*"]))
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_017,
            "openhuman.config_update_autonomy_settings",
            json!({ "level": "reckless" }),
        )
        .await,
        "update_autonomy_settings invalid level",
        "invalid autonomy level",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_018,
            "openhuman.config_update_model_settings",
            json!({
                "cloud_providers": [{
                    "slug": "",
                    "endpoint": "http://127.0.0.1:19999/v1",
                    "auth_style": "bearer"
                }]
            }),
        )
        .await,
        "update_model_settings empty cloud provider slug",
        "cloud provider slug must not be empty",
    );
    assert_error_contains(
        &rpc(
            &harness.rpc_base,
            11_022,
            "openhuman.config_update_model_settings",
            json!({
                "cloud_providers": [{
                    "slug": "worker-a-invalid-auth-style",
                    "label": "Invalid Auth Style",
                    "endpoint": "http://127.0.0.1:19999/v1",
                    "auth_style": "magic"
                }]
            }),
        )
        .await,
        "update_model_settings invalid cloud provider auth_style",
        "unknown auth_style",
    );
    let filtered_reserved = rpc(
        &harness.rpc_base,
        11_023,
        "openhuman.config_update_model_settings",
        json!({
            "cloud_providers": [
                {
                    "slug": "openhuman",
                    "label": "Reserved OpenHuman",
                    "endpoint": "https://api.openhuman.ai/v1",
                    "auth_style": "openhuman_jwt"
                },
                {
                    "slug": "worker-a-valid-cloud",
                    "label": "Worker A Valid Cloud",
                    "endpoint": "http://127.0.0.1:19999/v1",
                    "auth_style": "none"
                }
            ]
        }),
    )
    .await;
    let filtered_payload = payload(
        &filtered_reserved,
        "update_model_settings filters reserved cloud provider",
    );
    let cloud_providers = filtered_payload
        .pointer("/config/cloud_providers")
        .and_then(Value::as_array)
        .expect("config snapshot should expose cloud_providers");
    assert!(
        cloud_providers
            .iter()
            .any(|provider| provider.get("slug").and_then(Value::as_str)
                == Some("worker-a-valid-cloud")),
        "valid cloud provider should survive reserved filtering: {filtered_payload}"
    );
    assert_eq!(
        cloud_providers
            .iter()
            .filter(|provider| provider.get("slug").and_then(Value::as_str) == Some("openhuman"))
            .count(),
        1,
        "reserved cloud providers already in config should be preserved once, not duplicated by echoed client payloads: {filtered_payload}"
    );

    harness.join.abort();
}

#[tokio::test]
async fn config_auto_approve_public_helper_persists_once_and_is_idempotent() {
    let _lock = env_lock_async().await;
    let tmp = tempdir().expect("tempdir");
    let home = tmp.path().join("home");
    let _guards = [
        EnvVarGuard::set_to_path("HOME", &home),
        EnvVarGuard::unset("OPENHUMAN_WORKSPACE"),
        EnvVarGuard::unset(APP_ENV_VAR),
        EnvVarGuard::unset(VITE_APP_ENV_VAR),
    ];

    openhuman_core::config::add_auto_approve_tool(
        &DispatchCtx::process_default(),
        "tool.config.round10",
    )
    .await
    .expect("add auto approve tool");
    openhuman_core::config::add_auto_approve_tool(
        &DispatchCtx::process_default(),
        "tool.config.round10",
    )
    .await
    .expect("idempotent auto approve tool");

    let loaded = Config::load_or_init()
        .await
        .expect("load config after auto approve helper");
    assert_eq!(
        loaded
            .autonomy
            .auto_approve
            .iter()
            .filter(|tool| tool.as_str() == "tool.config.round10")
            .count(),
        1
    );
}
