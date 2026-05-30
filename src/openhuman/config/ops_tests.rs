use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn reset_local_data_removes_current_dir_default_dir_and_marker() {
    let temp = tempdir().unwrap();
    let default_openhuman_dir = temp.path().join("default-openhuman");
    let current_openhuman_dir = temp.path().join("custom-openhuman");
    let marker = active_workspace_marker_path(&default_openhuman_dir);

    tokio::fs::create_dir_all(default_openhuman_dir.join("workspace"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(current_openhuman_dir.join("workspace"))
        .await
        .unwrap();
    tokio::fs::write(&marker, "config_dir = '/tmp/custom-openhuman'\n")
        .await
        .unwrap();

    let outcome = reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir)
        .await
        .unwrap();

    assert!(!current_openhuman_dir.exists());
    assert!(!default_openhuman_dir.exists());
    assert!(outcome
        .value
        .get("removed_paths")
        .and_then(|value| value.as_array())
        .is_some_and(|paths| !paths.is_empty()));
}

// ── env_flag_enabled ────────────────────────────────────────────

use crate::openhuman::config::TEST_ENV_LOCK as ENV_LOCK;

#[test]
fn env_flag_enabled_recognizes_truthy_forms() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let key = "OPENHUMAN_TEST_FLAG_A";
    for truthy in ["1", "true", "TRUE", "yes", "YES"] {
        unsafe {
            std::env::set_var(key, truthy);
        }
        assert!(env_flag_enabled(key), "{truthy} should be truthy");
    }
    for falsy in ["0", "false", "off", "", "No"] {
        unsafe {
            std::env::set_var(key, falsy);
        }
        assert!(!env_flag_enabled(key), "{falsy} should be falsy");
    }
    unsafe {
        std::env::remove_var(key);
    }
    assert!(!env_flag_enabled(key), "unset must be falsy");
}

// ── core_rpc_url_from_env ───────────────────────────────────────

#[test]
fn core_rpc_url_from_env_returns_default_when_unset() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
    }
    assert_eq!(core_rpc_url_from_env(), "http://127.0.0.1:7788/rpc");
}

#[test]
fn core_rpc_url_from_env_uses_override_when_set() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::set_var("OPENHUMAN_CORE_RPC_URL", "http://1.2.3.4:9999/rpc");
    }
    assert_eq!(core_rpc_url_from_env(), "http://1.2.3.4:9999/rpc");
    unsafe {
        std::env::remove_var("OPENHUMAN_CORE_RPC_URL");
    }
}

// ── Pure path helpers ──────────────────────────────────────────

#[test]
fn fallback_workspace_dir_ends_in_workspace_under_openhuman() {
    let p = fallback_workspace_dir();
    assert!(p.ends_with("workspace"));
    assert!(p
        .parent()
        .map(|d| d.ends_with(".openhuman"))
        .unwrap_or(false));
}

#[test]
fn default_openhuman_dir_ends_in_dot_openhuman() {
    let p = default_openhuman_dir();
    assert!(p.ends_with(".openhuman"));
}

#[test]
fn active_workspace_marker_path_is_under_default_dir() {
    let default_dir = std::path::Path::new("/tmp/openhuman-test");
    let marker = active_workspace_marker_path(default_dir);
    assert_eq!(marker, default_dir.join("active_workspace.toml"));
}

#[test]
fn config_openhuman_dir_returns_config_path_parent() {
    let mut cfg = Config::default();
    cfg.config_path = PathBuf::from("/tmp/xyz/config.toml");
    assert_eq!(config_openhuman_dir(&cfg), PathBuf::from("/tmp/xyz"));
}

#[cfg(windows)]
#[test]
fn reset_local_data_remove_error_explains_windows_file_locks() {
    let err = std::io::Error::from_raw_os_error(32);
    let msg =
        reset_local_data_remove_error(std::path::Path::new("C:\\Users\\me\\.openhuman"), &err);

    assert!(msg.contains("locked by another OpenHuman window or process"));
    assert!(msg.contains("Close all OpenHuman windows and try again"));
}

#[cfg(windows)]
#[test]
fn reset_local_data_remove_error_explains_windows_lock_violation() {
    let err = std::io::Error::from_raw_os_error(33);
    let msg =
        reset_local_data_remove_error(std::path::Path::new("C:\\Users\\me\\.openhuman"), &err);

    assert!(msg.contains("locked by another OpenHuman window or process"));
    assert!(msg.contains("Close all OpenHuman windows and try again"));
}

// ── get_runtime_flags / set_browser_allow_all ─────────────────

#[test]
fn get_runtime_flags_reads_env_overrides() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
    }
    let flags = get_runtime_flags();
    // Just exercise the path — we don't assume anything about
    // what other tests in the suite may have set.
    let _ = flags.value;
}

#[test]
fn set_browser_allow_all_rejects_enable_without_operator_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let before = std::env::var(BROWSER_ALLOW_ALL_ENV).ok();
    let before_override = std::env::var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV).ok();

    unsafe {
        std::env::remove_var(BROWSER_ALLOW_ALL_ENV);
        std::env::remove_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV);
    }

    let err = set_browser_allow_all(true).expect_err("runtime enable should require override");
    assert!(
        err.contains("Refusing to enable OPENHUMAN_BROWSER_ALLOW_ALL via RPC"),
        "unexpected error: {err}"
    );
    assert!(!env_flag_enabled(BROWSER_ALLOW_ALL_ENV));

    unsafe {
        match before {
            Some(v) => std::env::set_var(BROWSER_ALLOW_ALL_ENV, v),
            None => std::env::remove_var(BROWSER_ALLOW_ALL_ENV),
        }
        match before_override {
            Some(v) => std::env::set_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV, v),
            None => std::env::remove_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV),
        }
    }
}

#[test]
fn set_browser_allow_all_toggles_env_var_when_operator_override_is_set() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let before = std::env::var(BROWSER_ALLOW_ALL_ENV).ok();
    let before_override = std::env::var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV).ok();

    unsafe {
        std::env::remove_var(BROWSER_ALLOW_ALL_ENV);
        std::env::set_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV, "1");
    }

    let enable_outcome = set_browser_allow_all(true).expect("override should allow runtime enable");
    assert_eq!(enable_outcome.logs.len(), 1);
    let enable_log = &enable_outcome.logs[0];
    assert!(
        enable_log.contains("[SECURITY]"),
        "enable log should be audit-tagged: {enable_log}"
    );
    assert!(
        enable_log.contains("enabled"),
        "enable log should mention enabled state: {enable_log}"
    );
    assert!(enable_outcome.value.browser_allow_all);
    assert!(env_flag_enabled(BROWSER_ALLOW_ALL_ENV));

    let disable_outcome = set_browser_allow_all(false).expect("runtime disable should always work");
    assert_eq!(disable_outcome.logs.len(), 1);
    let disable_log = &disable_outcome.logs[0];
    assert!(
        disable_log.contains("[SECURITY]"),
        "disable log should be audit-tagged: {disable_log}"
    );
    assert!(
        disable_log.contains("disabled"),
        "disable log should mention disabled state: {disable_log}"
    );
    assert!(!disable_outcome.value.browser_allow_all);
    assert!(!env_flag_enabled(BROWSER_ALLOW_ALL_ENV));

    unsafe {
        match before {
            Some(v) => std::env::set_var(BROWSER_ALLOW_ALL_ENV, v),
            None => std::env::remove_var(BROWSER_ALLOW_ALL_ENV),
        }
        match before_override {
            Some(v) => std::env::set_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV, v),
            None => std::env::remove_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV),
        }
    }
}

#[test]
fn set_browser_allow_all_disable_does_not_require_operator_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let before = std::env::var(BROWSER_ALLOW_ALL_ENV).ok();
    let before_override = std::env::var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV).ok();

    unsafe {
        std::env::set_var(BROWSER_ALLOW_ALL_ENV, "1");
        std::env::remove_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV);
    }

    let disable_outcome =
        set_browser_allow_all(false).expect("runtime disable should not require override");
    assert!(
        disable_outcome.logs[0].contains("[SECURITY]"),
        "disable log should be audit-tagged: {:?}",
        disable_outcome.logs
    );
    assert!(!disable_outcome.value.browser_allow_all);
    assert!(!env_flag_enabled(BROWSER_ALLOW_ALL_ENV));

    unsafe {
        match before {
            Some(v) => std::env::set_var(BROWSER_ALLOW_ALL_ENV, v),
            None => std::env::remove_var(BROWSER_ALLOW_ALL_ENV),
        }
        match before_override {
            Some(v) => std::env::set_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV, v),
            None => std::env::remove_var(BROWSER_ALLOW_ALL_RPC_ENABLE_ENV),
        }
    }
}

// ── snapshot_config_json ───────────────────────────────────────

#[test]
fn snapshot_config_json_emits_config_and_workspace_and_config_path() {
    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");

    let snap = snapshot_config_json(&cfg).expect("snapshot should succeed");
    assert!(snap.get("config").is_some());
    assert!(snap.get("workspace_dir").is_some());
    assert!(snap.get("config_path").is_some());
    // Workspace + config paths must point at our tempdir.
    let ws = snap["workspace_dir"].as_str().unwrap_or("");
    assert!(ws.contains(tmp.path().to_str().unwrap_or("")));
}

// ── agent_server_status ────────────────────────────────────────

#[test]
fn agent_server_status_exposes_running_and_url() {
    let outcome = agent_server_status();
    assert!(outcome.value.get("running").is_some());
    assert!(outcome.value.get("url").is_some());
}

// ── workspace_onboarding_flag_exists ───────────────────────────

#[test]
fn workspace_onboarding_flag_exists_returns_false_for_fresh_workspace() {
    let tmp = tempdir().unwrap();
    let res = workspace_onboarding_flag_exists(tmp.path().join("workspace"), "onboarding.done")
        .expect("flag check ok");
    assert_eq!(res.value, false);
}

#[test]
fn workspace_onboarding_flag_exists_rejects_invalid_flag_names() {
    let tmp = tempdir().unwrap();
    for bad in ["", "   ", "a/b", "a\\b", "..", "foo/.."] {
        let err = workspace_onboarding_flag_exists(tmp.path().join("workspace"), bad).unwrap_err();
        assert!(
            err.contains("Invalid onboarding flag"),
            "name `{bad}`: {err}"
        );
    }
}

#[test]
fn workspace_onboarding_flag_exists_true_when_file_present() {
    let tmp = tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::write(ws.join("onboarding.done"), "").unwrap();
    let res = workspace_onboarding_flag_exists(ws, "onboarding.done").expect("flag check ok");
    assert_eq!(res.value, true);
}

// ── apply_*_settings ─────────────────────────────────────────

fn tmp_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");
    std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
    cfg
}

#[tokio::test]
async fn apply_model_settings_updates_fields_and_persists_snapshot() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let patch = ModelSettingsPatch {
        api_url: Some("https://api.example.test".into()),
        inference_url: None,
        api_key: None,
        default_model: Some("gpt-4o".into()),
        default_temperature: Some(0.25),
        model_routes: None,
        ..Default::default()
    };
    let outcome = apply_model_settings(&mut cfg, patch).await.expect("apply");
    assert_eq!(cfg.api_url.as_deref(), Some("https://api.example.test"));
    assert_eq!(cfg.default_model.as_deref(), Some("gpt-4o"));
    assert!((cfg.default_temperature - 0.25).abs() < f64::EPSILON);
    assert_eq!(
        outcome.value["config"]["api_url"],
        "https://api.example.test"
    );
}

#[tokio::test]
async fn apply_search_settings_sets_and_clears_allowed_domains() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    // Explicit host list is trimmed, blanks dropped, sorted + de-duped.
    let patch = SearchSettingsPatch {
        allowed_domains: Some(vec![
            " reuters.com ".into(),
            "reuters.com".into(),
            String::new(),
            "github.com".into(),
        ]),
        ..Default::default()
    };
    apply_search_settings(&mut cfg, patch).await.expect("apply");
    assert_eq!(
        cfg.http_request.allowed_domains,
        vec!["github.com".to_string(), "reuters.com".to_string()]
    );

    // allow_all = true collapses the list to the wildcard.
    let patch = SearchSettingsPatch {
        allow_all: Some(true),
        ..Default::default()
    };
    apply_search_settings(&mut cfg, patch).await.expect("apply");
    assert_eq!(cfg.http_request.allowed_domains, vec!["*".to_string()]);

    // allow_all = false drops the wildcard (explicit hosts only / blocked).
    let patch = SearchSettingsPatch {
        allow_all: Some(false),
        ..Default::default()
    };
    apply_search_settings(&mut cfg, patch).await.expect("apply");
    assert!(cfg.http_request.allowed_domains.is_empty());
}

#[tokio::test]
async fn apply_search_settings_accepts_disabled_engine() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    apply_search_settings(
        &mut cfg,
        SearchSettingsPatch {
            engine: Some("disabled".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect("apply disabled search engine");

    assert_eq!(cfg.search.engine, "disabled");
    assert_eq!(
        cfg.search.effective_engine(),
        crate::openhuman::config::SearchEngine::Disabled
    );
}

#[tokio::test]
async fn apply_search_settings_rejects_unknown_search_engine() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let err = apply_search_settings(
        &mut cfg,
        SearchSettingsPatch {
            engine: Some("unknown".to_string()),
            ..Default::default()
        },
    )
    .await
    .expect_err("unknown engine should be rejected");

    assert!(err.contains("disabled/managed/parallel/brave/querit"));
}

#[tokio::test]
async fn apply_model_settings_stores_api_key_and_clears_when_empty() {
    // #1342: custom OpenAI-compatible providers — api_key must round-trip
    // through `apply_model_settings` and clear when an empty string is sent.
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let set = ModelSettingsPatch {
        api_url: Some("https://llm.example.test/v1".into()),
        inference_url: None,
        api_key: Some("  sk-test-1234  ".into()),
        default_model: Some("gpt-4o-mini".into()),
        default_temperature: None,
        model_routes: None,
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, set).await.expect("set");
    assert_eq!(cfg.api_key.as_deref(), Some("sk-test-1234"));

    let clear = ModelSettingsPatch {
        api_url: None,
        inference_url: None,
        api_key: Some("".into()),
        default_model: None,
        default_temperature: None,
        model_routes: None,
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, clear).await.expect("clear");
    assert!(cfg.api_key.is_none());
    // Other fields must not be disturbed by a key-only clear.
    assert_eq!(cfg.api_url.as_deref(), Some("https://llm.example.test/v1"));
    assert_eq!(cfg.default_model.as_deref(), Some("gpt-4o-mini"));
}

#[tokio::test]
async fn apply_model_settings_replaces_model_routes_when_some_and_keeps_when_none() {
    // #1342: switching providers writes role->model routes; switching back to
    // OpenHuman sends an empty vec to wipe them. Omitting the field leaves
    // existing routes alone.
    use crate::openhuman::config::ModelRouteConfig;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let set_routes = ModelSettingsPatch {
        api_url: None,
        inference_url: None,
        api_key: None,
        default_model: None,
        default_temperature: None,
        model_routes: Some(vec![
            ModelRouteConfig {
                hint: "reasoning".into(),
                model: "o1".into(),
            },
            ModelRouteConfig {
                hint: "agentic".into(),
                model: "gpt-4o".into(),
            },
        ]),
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, set_routes)
        .await
        .expect("set");
    assert_eq!(cfg.model_routes.len(), 2);
    assert_eq!(cfg.model_routes[0].hint, "reasoning");

    // None — leave routes alone.
    let touch_other = ModelSettingsPatch {
        api_url: Some("https://x.test/v1".into()),
        inference_url: None,
        api_key: None,
        default_model: None,
        default_temperature: None,
        model_routes: None,
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, touch_other)
        .await
        .expect("touch");
    assert_eq!(cfg.model_routes.len(), 2);
    assert_eq!(cfg.api_url.as_deref(), Some("https://x.test/v1"));

    // Empty vec — clear.
    let clear_routes = ModelSettingsPatch {
        api_url: None,
        inference_url: None,
        api_key: None,
        default_model: None,
        default_temperature: None,
        model_routes: Some(vec![]),
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, clear_routes)
        .await
        .expect("clear");
    assert!(cfg.model_routes.is_empty());
}

#[tokio::test]
async fn apply_model_settings_empty_strings_clear_optional_fields() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.default_model = Some("prev-model".into());
    let patch = ModelSettingsPatch {
        api_url: Some("".into()),
        inference_url: None,
        api_key: None,
        default_model: Some("".into()),
        default_temperature: None,
        model_routes: None,
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, patch).await.expect("apply");
    assert!(cfg.api_url.is_none());
    assert!(cfg.default_model.is_none());
}

#[tokio::test]
async fn apply_model_settings_preserves_existing_reserved_slug_cloud_providers() {
    // Sentry TAURI-RUST-5 regression. The migration
    // `unify_ai_provider_settings` seeds an "openhuman"-slug entry into
    // `cloud_providers`. The frontend echoes the full cloud_providers
    // list back on every settings save, but the schema handlers filter
    // out reserved-slug entries before passing them through. Without
    // this preservation step the filtered patch would silently delete
    // the built-in entry — losing the `primary_cloud` referent and
    // breaking inference routing.
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    // Simulate the post-migration state: a built-in "openhuman" entry plus
    // a user-added custom provider.
    cfg.cloud_providers = vec![
        CloudProviderCreds {
            id: "openhuman-builtin".into(),
            slug: "openhuman".into(),
            label: "OpenHuman".into(),
            endpoint: "https://api.tinyhumans.ai".into(),
            auth_style: AuthStyle::OpenhumanJwt,
            default_model: Some("reasoning-v1".into()),
            ..Default::default()
        },
        CloudProviderCreds {
            id: "myopenai-1".into(),
            slug: "myopenai".into(),
            label: "My OpenAI".into(),
            endpoint: "https://api.openai.com".into(),
            auth_style: AuthStyle::Bearer,
            default_model: Some("gpt-4o".into()),
            ..Default::default()
        },
    ];

    // The patch arrives from the schema handler with the "openhuman"
    // entry already filtered out (the schema handler drops reserved
    // slugs silently). Only the user's custom provider is present, with
    // the user's edit applied.
    let patch = ModelSettingsPatch {
        cloud_providers: Some(vec![CloudProviderCreds {
            id: "myopenai-1".into(),
            slug: "myopenai".into(),
            label: "My OpenAI (edited)".into(),
            endpoint: "https://api.openai.com/v1".into(),
            auth_style: AuthStyle::Bearer,
            default_model: Some("gpt-4o-mini".into()),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let _ = apply_model_settings(&mut cfg, patch).await.expect("apply");

    // The user's edit is applied.
    let myopenai = cfg
        .cloud_providers
        .iter()
        .find(|e| e.slug == "myopenai")
        .expect("myopenai entry survives");
    assert_eq!(myopenai.label, "My OpenAI (edited)");
    assert_eq!(myopenai.default_model.as_deref(), Some("gpt-4o-mini"));

    // And the built-in "openhuman" entry is still there.
    let openhuman = cfg
        .cloud_providers
        .iter()
        .find(|e| e.slug == "openhuman")
        .expect("openhuman built-in must be preserved across saves");
    assert_eq!(openhuman.id, "openhuman-builtin");
    assert_eq!(openhuman.endpoint, "https://api.tinyhumans.ai");
}

#[tokio::test]
async fn apply_model_settings_does_not_double_add_reserved_entries() {
    // Defensive: if a caller bypasses the schema handler (CLI / tests) and
    // includes a reserved-slug entry in the patch, the preservation logic
    // must not double-add it.
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.cloud_providers = vec![CloudProviderCreds {
        id: "openhuman-stored".into(),
        slug: "openhuman".into(),
        label: "OpenHuman (stored)".into(),
        endpoint: "https://api.tinyhumans.ai".into(),
        auth_style: AuthStyle::OpenhumanJwt,
        default_model: Some("reasoning-v1".into()),
        ..Default::default()
    }];

    let patch = ModelSettingsPatch {
        cloud_providers: Some(vec![CloudProviderCreds {
            id: "openhuman-from-patch".into(),
            slug: "openhuman".into(),
            label: "OpenHuman (from patch)".into(),
            endpoint: "https://api.tinyhumans.ai".into(),
            auth_style: AuthStyle::OpenhumanJwt,
            default_model: Some("reasoning-v1".into()),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let _ = apply_model_settings(&mut cfg, patch).await.expect("apply");

    // Exactly one "openhuman" entry survives; the patch's version wins
    // (since it was already in `providers` before preservation ran).
    let count = cfg
        .cloud_providers
        .iter()
        .filter(|e| e.slug == "openhuman")
        .count();
    assert_eq!(count, 1, "no duplicate reserved-slug entries");
    let entry = cfg
        .cloud_providers
        .iter()
        .find(|e| e.slug == "openhuman")
        .unwrap();
    assert_eq!(entry.id, "openhuman-from-patch");
}

#[tokio::test]
async fn apply_memory_settings_updates_all_provided_fields() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let patch = MemorySettingsPatch {
        backend: Some("sqlite".into()),
        auto_save: Some(true),
        embedding_provider: Some("ollama".into()),
        embedding_model: Some("nomic".into()),
        embedding_dimensions: Some(768),
        memory_window: Some("extended".into()),
    };
    let _ = apply_memory_settings(&mut cfg, patch).await.expect("apply");
    assert_eq!(cfg.memory.backend, "sqlite");
    assert!(cfg.memory.auto_save);
    assert_eq!(cfg.memory.embedding_provider, "ollama");
    assert_eq!(cfg.memory.embedding_model, "nomic");
    assert_eq!(cfg.memory.embedding_dimensions, 768);
    assert_eq!(
        cfg.agent.memory_window,
        Some(crate::openhuman::config::schema::MemoryContextWindow::Extended)
    );
}

#[tokio::test]
async fn apply_autonomy_settings_updates_action_budget() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.autonomy.max_actions_per_hour = 20;

    let outcome = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(64),
            ..Default::default()
        },
    )
    .await
    .expect("apply autonomy settings");

    assert_eq!(cfg.autonomy.max_actions_per_hour, 64);
    assert_eq!(
        outcome.value["config"]["autonomy"]["max_actions_per_hour"],
        serde_json::json!(64)
    );
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("autonomy settings saved to")));
}

#[tokio::test]
async fn apply_memory_settings_ignores_unknown_memory_window_label() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.agent.memory_window = Some(crate::openhuman::config::schema::MemoryContextWindow::Balanced);
    let original = cfg.agent.memory_window;
    let patch = MemorySettingsPatch {
        memory_window: Some("ginormous".into()),
        ..MemorySettingsPatch::default()
    };
    let _ = apply_memory_settings(&mut cfg, patch).await.expect("apply");
    assert_eq!(cfg.agent.memory_window, original);
}

#[tokio::test]
async fn apply_memory_settings_round_trips_all_window_labels() {
    use crate::openhuman::config::schema::MemoryContextWindow;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let windows: [MemoryContextWindow; 4] = [
        MemoryContextWindow::Minimal,
        MemoryContextWindow::Balanced,
        MemoryContextWindow::Extended,
        MemoryContextWindow::Maximum,
    ];
    for window in windows {
        let patch = MemorySettingsPatch {
            memory_window: Some(window.as_str().to_string()),
            ..MemorySettingsPatch::default()
        };
        apply_memory_settings(&mut cfg, patch).await.expect("apply");
        assert_eq!(cfg.agent.memory_window, Some(window));
    }
}

#[tokio::test]
async fn apply_runtime_settings_updates_kind_and_reasoning() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let patch = RuntimeSettingsPatch {
        kind: Some("desktop".into()),
        reasoning_enabled: Some(true),
    };
    let _ = apply_runtime_settings(&mut cfg, patch)
        .await
        .expect("apply");
    assert_eq!(cfg.runtime.kind, "desktop");
    assert_eq!(cfg.runtime.reasoning_enabled, Some(true));
}

#[tokio::test]
async fn apply_browser_settings_updates_enabled_flag() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.browser.enabled = false;
    let _ = apply_browser_settings(
        &mut cfg,
        BrowserSettingsPatch {
            enabled: Some(true),
        },
    )
    .await
    .expect("apply");
    assert!(cfg.browser.enabled);
}

#[tokio::test]
async fn apply_local_ai_settings_updates_lm_studio_provider_fields() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.local_ai.model_id = "old-default".into();
    cfg.local_ai.chat_model_id = "old-chat".into();

    let patch = LocalAiSettingsPatch {
        runtime_enabled: Some(true),
        opt_in_confirmed: Some(true),
        provider: Some("lm-studio".into()),
        base_url: Some(" http://localhost:1234/v1/ ".into()),
        model_id: Some(" local-default ".into()),
        chat_model_id: Some(" local-chat ".into()),
        usage_embeddings: Some(true),
        usage_heartbeat: Some(true),
        usage_learning_reflection: Some(false),
        usage_subconscious: Some(true),
    };

    let outcome = apply_local_ai_settings(&mut cfg, patch)
        .await
        .expect("apply local ai");

    assert!(cfg.local_ai.runtime_enabled);
    assert!(cfg.local_ai.opt_in_confirmed);
    assert_eq!(cfg.local_ai.provider, "lm_studio");
    assert_eq!(
        cfg.local_ai.base_url.as_deref(),
        Some("http://localhost:1234/v1/")
    );
    assert_eq!(cfg.local_ai.model_id, "local-default");
    assert_eq!(cfg.local_ai.chat_model_id, "local-chat");
    assert!(cfg.local_ai.usage.embeddings);
    assert!(cfg.local_ai.usage.heartbeat);
    assert!(!cfg.local_ai.usage.learning_reflection);
    assert!(cfg.local_ai.usage.subconscious);
    assert_eq!(outcome.value["config"]["local_ai"]["provider"], "lm_studio");

    let clear_and_fallback = LocalAiSettingsPatch {
        provider: Some("unknown-provider".into()),
        base_url: Some("   ".into()),
        model_id: Some("   ".into()),
        chat_model_id: Some("".into()),
        ..LocalAiSettingsPatch::default()
    };
    apply_local_ai_settings(&mut cfg, clear_and_fallback)
        .await
        .expect("clear local ai");

    assert_eq!(cfg.local_ai.provider, "ollama");
    assert!(cfg.local_ai.base_url.is_none());
    assert_eq!(cfg.local_ai.model_id, "");
    assert_eq!(cfg.local_ai.chat_model_id, "");
}

#[tokio::test]
async fn apply_analytics_settings_updates_enabled() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let _ = apply_analytics_settings(
        &mut cfg,
        AnalyticsSettingsPatch {
            enabled: Some(false),
        },
    )
    .await
    .expect("apply");
    assert!(!cfg.observability.analytics_enabled);
}

#[tokio::test]
async fn apply_meet_settings_updates_handoff_flag() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    // Default is OFF for a fresh config (issue #1299).
    assert!(
        !cfg.meet.auto_orchestrator_handoff,
        "fresh config must start with auto_orchestrator_handoff=false"
    );
    // Flip ON.
    let _ = apply_meet_settings(
        &mut cfg,
        MeetSettingsPatch {
            auto_orchestrator_handoff: Some(true),
        },
    )
    .await
    .expect("apply on");
    assert!(cfg.meet.auto_orchestrator_handoff);
    // Flip OFF again — covers the off-after-on path.
    let _ = apply_meet_settings(
        &mut cfg,
        MeetSettingsPatch {
            auto_orchestrator_handoff: Some(false),
        },
    )
    .await
    .expect("apply off");
    assert!(!cfg.meet.auto_orchestrator_handoff);
    // No-op patch must not change the flag.
    let prior = cfg.meet.auto_orchestrator_handoff;
    let _ = apply_meet_settings(
        &mut cfg,
        MeetSettingsPatch {
            auto_orchestrator_handoff: None,
        },
    )
    .await
    .expect("apply noop");
    assert_eq!(prior, cfg.meet.auto_orchestrator_handoff);
}

#[tokio::test]
async fn get_config_snapshot_wraps_snapshot_in_rpc_outcome() {
    let tmp = tempdir().unwrap();
    let cfg = tmp_config(&tmp);
    let outcome = get_config_snapshot(&cfg).await.expect("snapshot");
    assert!(outcome.value.get("config").is_some());
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("config loaded from")));
}

// ── Dictation / voice_server settings patches ─────────────────

#[tokio::test]
async fn load_and_apply_dictation_settings_rejects_invalid_activation_mode() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let patch = DictationSettingsPatch {
        enabled: None,
        hotkey: None,
        activation_mode: Some("not-a-mode".into()),
        llm_refinement: None,
        streaming: None,
        streaming_interval_ms: None,
    };
    let err = load_and_apply_dictation_settings(patch).await.unwrap_err();
    assert!(err.contains("invalid activation_mode"));
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn load_and_apply_voice_server_settings_rejects_invalid_activation_mode() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let patch = VoiceServerSettingsPatch {
        auto_start: None,
        hotkey: None,
        activation_mode: Some("hold".into()),
        skip_cleanup: None,
        min_duration_secs: None,
        silence_threshold: None,
        custom_dictionary: None,
    };
    let err = load_and_apply_voice_server_settings(patch)
        .await
        .unwrap_err();
    assert!(err.contains("invalid activation_mode"));
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn load_and_apply_dictation_settings_accepts_valid_modes() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    for mode in ["toggle", "push"] {
        let patch = DictationSettingsPatch {
            enabled: Some(true),
            hotkey: Some("cmd+d".into()),
            activation_mode: Some(mode.into()),
            llm_refinement: Some(false),
            streaming: Some(false),
            streaming_interval_ms: Some(500),
        };
        assert!(
            load_and_apply_dictation_settings(patch).await.is_ok(),
            "mode `{mode}` should be accepted"
        );
    }
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn load_and_apply_voice_server_settings_accepts_valid_modes_and_clamps() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    // Negative min_duration_secs and silence_threshold should be clamped to 0.
    let patch = VoiceServerSettingsPatch {
        auto_start: Some(true),
        hotkey: Some("fn".into()),
        activation_mode: Some("tap".into()),
        skip_cleanup: Some(false),
        min_duration_secs: Some(-5.0),
        silence_threshold: Some(-1.0),
        custom_dictionary: Some(vec!["term".into()]),
    };
    let outcome = load_and_apply_voice_server_settings(patch)
        .await
        .expect("ok");
    assert!(
        outcome.value["config"]["voice_server"]["min_duration_secs"]
            .as_f64()
            .unwrap_or(-1.0)
            >= 0.0
    );
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

// ── get_* via env override ─────────────────────────────────────

#[tokio::test]
async fn get_dictation_settings_reads_from_loaded_config() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let outcome = get_dictation_settings().await.expect("ok");
    assert!(outcome.value.get("enabled").is_some());
    assert!(outcome.value.get("hotkey").is_some());
    assert!(outcome.value.get("streaming_interval_ms").is_some());
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn get_voice_server_settings_reads_from_loaded_config() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let outcome = get_voice_server_settings().await.expect("ok");
    assert!(outcome.value.get("auto_start").is_some());
    assert!(outcome.value.get("custom_dictionary").is_some());
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn get_onboarding_completed_reads_from_loaded_config() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let outcome = get_onboarding_completed().await.expect("ok");
    // Default value — either true or false is fine; we just verify the call path.
    let _ = outcome.value;
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn load_and_resolve_api_url_returns_api_url_in_response() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let outcome = load_and_resolve_api_url().await.expect("ok");
    assert!(outcome.value.get("api_url").is_some());
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn workspace_onboarding_flag_resolve_rejects_invalid_and_defaults() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let err = workspace_onboarding_flag_resolve(Some("a/b".into()), "done")
        .await
        .unwrap_err();
    assert!(err.contains("Invalid onboarding flag"));

    // Happy path: default name on a fresh workspace → file doesn't exist.
    let outcome = workspace_onboarding_flag_resolve(None, "onboarding.done")
        .await
        .expect("ok");
    let _ = outcome.value;
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn workspace_onboarding_flag_set_rejects_invalid_names() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    for bad in ["", "   ", "a/b", "a\\b", ".."] {
        let err = workspace_onboarding_flag_set(Some(bad.into()), "default", true)
            .await
            .unwrap_err();
        assert!(err.contains("Invalid onboarding flag"), "name {bad}: {err}");
    }
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn workspace_onboarding_flag_set_round_trip() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    // Create flag
    let created = workspace_onboarding_flag_set(Some("onboarding.done".into()), "default", true)
        .await
        .expect("create");
    assert!(created.value);
    // Remove flag
    let removed = workspace_onboarding_flag_set(Some("onboarding.done".into()), "default", false)
        .await
        .expect("remove");
    assert!(!removed.value);
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn apply_model_settings_trims_and_clears_optional_provider_fields() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let set = ModelSettingsPatch {
        inference_url: Some(" https://llm.example.test/v1 ".into()),
        primary_cloud: Some(" provider-a ".into()),
        reasoning_provider: Some(" provider-reasoning ".into()),
        agentic_provider: Some(" provider-agentic ".into()),
        coding_provider: Some(" provider-coding ".into()),
        memory_provider: Some(" provider-memory ".into()),
        embeddings_provider: Some(" provider-embed ".into()),
        heartbeat_provider: Some(" provider-heartbeat ".into()),
        learning_provider: Some(" provider-learning ".into()),
        subconscious_provider: Some(" provider-sub ".into()),
        ..Default::default()
    };
    apply_model_settings(&mut cfg, set)
        .await
        .expect("set providers");
    assert_eq!(
        cfg.inference_url.as_deref(),
        Some("https://llm.example.test/v1")
    );
    assert_eq!(cfg.primary_cloud.as_deref(), Some("provider-a"));
    assert_eq!(
        cfg.reasoning_provider.as_deref(),
        Some("provider-reasoning")
    );
    assert_eq!(cfg.subconscious_provider.as_deref(), Some("provider-sub"));

    let clear = ModelSettingsPatch {
        inference_url: Some("   ".into()),
        primary_cloud: Some("".into()),
        reasoning_provider: Some(" ".into()),
        agentic_provider: Some(" ".into()),
        coding_provider: Some(" ".into()),
        memory_provider: Some(" ".into()),
        embeddings_provider: Some(" ".into()),
        heartbeat_provider: Some(" ".into()),
        learning_provider: Some(" ".into()),
        subconscious_provider: Some(" ".into()),
        ..Default::default()
    };
    apply_model_settings(&mut cfg, clear)
        .await
        .expect("clear providers");
    assert!(cfg.inference_url.is_none());
    assert!(cfg.primary_cloud.is_none());
    assert!(cfg.reasoning_provider.is_none());
    assert!(cfg.agentic_provider.is_none());
    assert!(cfg.coding_provider.is_none());
    assert!(cfg.memory_provider.is_none());
    assert!(cfg.embeddings_provider.is_none());
    assert!(cfg.heartbeat_provider.is_none());
    assert!(cfg.learning_provider.is_none());
    assert!(cfg.subconscious_provider.is_none());
}

#[tokio::test]
async fn apply_screen_intelligence_settings_clamps_baseline_fps() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    apply_screen_intelligence_settings(
        &mut cfg,
        ScreenIntelligenceSettingsPatch {
            baseline_fps: Some(99.0),
            ..Default::default()
        },
    )
    .await
    .expect("high clamp");
    assert!((cfg.screen_intelligence.baseline_fps - 30.0).abs() < f32::EPSILON);

    apply_screen_intelligence_settings(
        &mut cfg,
        ScreenIntelligenceSettingsPatch {
            baseline_fps: Some(0.01),
            ..Default::default()
        },
    )
    .await
    .expect("low clamp");
    assert!((cfg.screen_intelligence.baseline_fps - 0.2).abs() < f32::EPSILON);
}

// ── apply_autonomy_settings ────────────────────────────────────

#[tokio::test]
async fn apply_autonomy_settings_persists_max_actions_per_hour() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let outcome = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(200),
            ..Default::default()
        },
    )
    .await
    .expect("apply");
    assert_eq!(cfg.autonomy.max_actions_per_hour, 200);
    // Snapshot returned so the caller can echo the saved state.
    assert!(outcome.value.get("config").is_some());
    // Round-trip from disk: reload the saved TOML and confirm.
    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("max_actions_per_hour = 200"),
        "expected TOML to contain max_actions_per_hour = 200, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn apply_autonomy_settings_no_op_when_patch_empty() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let prior = cfg.autonomy.max_actions_per_hour;
    let _ = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: None,
            ..Default::default()
        },
    )
    .await
    .expect("apply noop");
    assert_eq!(cfg.autonomy.max_actions_per_hour, prior);
}

#[tokio::test]
async fn apply_autonomy_settings_rejects_zero() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let err = apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(0),
            ..Default::default()
        },
    )
    .await
    .unwrap_err();
    assert!(
        err.contains("at least 1"),
        "expected validation error, got: {err}"
    );
}

#[tokio::test]
async fn apply_autonomy_settings_accepts_unlimited_sentinel() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // u32::MAX is the new "unlimited" sentinel exposed by the UI as a
    // preset. The upper cap was lifted in the same PR that defaulted
    // fresh installs to u32::MAX; anything in [1, u32::MAX] should now
    // round-trip cleanly.
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            max_actions_per_hour: Some(u32::MAX),
            ..Default::default()
        },
    )
    .await
    .expect("u32::MAX (unlimited) should round-trip");
    assert_eq!(cfg.autonomy.max_actions_per_hour, u32::MAX);
}

#[tokio::test]
async fn load_and_apply_autonomy_settings_roundtrip() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    let patch = AutonomySettingsPatch {
        max_actions_per_hour: Some(500),
        ..Default::default()
    };
    let outcome = load_and_apply_autonomy_settings(patch)
        .await
        .expect("apply");
    assert!(outcome.value.get("config").is_some());

    // Reload from scratch and confirm the saved value sticks.
    let reloaded = load_config_with_timeout().await.expect("reload");
    assert_eq!(reloaded.autonomy.max_actions_per_hour, 500);

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

#[tokio::test]
async fn apply_autonomy_settings_replaces_auto_approve() {
    // ENV_LOCK serializes the `live_policy::reload_from` triggered by
    // `apply_autonomy_settings` against other live-policy-touching tests.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    apply_autonomy_settings(
        &mut cfg,
        AutonomySettingsPatch {
            auto_approve: Some(vec!["shell".into(), "curl".into()]),
            ..Default::default()
        },
    )
    .await
    .expect("apply auto_approve");
    assert_eq!(cfg.autonomy.auto_approve, vec!["shell", "curl"]);
    // Persisted to the TOML, not just held in memory.
    let on_disk = tokio::fs::read_to_string(&cfg.config_path).await.unwrap();
    assert!(
        on_disk.contains("auto_approve") && on_disk.contains("shell") && on_disk.contains("curl"),
        "auto_approve allowlist should round-trip to TOML, got:\n{on_disk}"
    );
}

#[tokio::test]
async fn add_auto_approve_tool_appends_then_dedupes() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }

    add_auto_approve_tool("git_operations")
        .await
        .expect("first add");
    // Idempotent: a second add of the same tool must not create a duplicate.
    add_auto_approve_tool("git_operations")
        .await
        .expect("second add (idempotent)");

    let reloaded = load_config_with_timeout().await.expect("reload");
    let hits = reloaded
        .autonomy
        .auto_approve
        .iter()
        .filter(|t| t.as_str() == "git_operations")
        .count();
    assert_eq!(
        hits, 1,
        "tool must appear exactly once after duplicate adds"
    );

    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}
