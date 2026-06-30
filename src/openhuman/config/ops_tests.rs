use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn reset_local_data_removes_active_user_and_markers_only() {
    let temp = tempdir().unwrap();
    let default_openhuman_dir = temp.path().join("default-openhuman");
    // Active user lives under the shared root's `users/` tree, mirroring the
    // real layout (`~/.openhuman/users/<id>`).
    let current_openhuman_dir = default_openhuman_dir.join("users").join("active-user");
    let workspace_marker = active_workspace_marker_path(&default_openhuman_dir);
    let user_marker = crate::openhuman::config::active_user_marker_path(&default_openhuman_dir);

    tokio::fs::create_dir_all(current_openhuman_dir.join("workspace"))
        .await
        .unwrap();
    tokio::fs::write(&workspace_marker, "config_dir = 'users/active-user'\n")
        .await
        .unwrap();
    tokio::fs::write(&user_marker, "user_id = 'active-user'\n")
        .await
        .unwrap();

    let outcome = reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir)
        .await
        .unwrap();

    // Active user's slice and both shared markers are gone …
    assert!(!current_openhuman_dir.exists());
    assert!(!workspace_marker.exists());
    assert!(!user_marker.exists());
    // … but the shared root itself survives.
    assert!(default_openhuman_dir.exists());
    assert!(outcome
        .value
        .get("removed_paths")
        .and_then(|value| value.as_array())
        .is_some_and(|paths| !paths.is_empty()));
}

#[tokio::test]
async fn reset_local_data_preserves_sibling_users() {
    let temp = tempdir().unwrap();
    let default_openhuman_dir = temp.path().join("default-openhuman");
    let current_openhuman_dir = default_openhuman_dir.join("users").join("active-user");
    let sibling_user_dir = default_openhuman_dir.join("users").join("other-user");
    let sibling_file = sibling_user_dir.join("config.toml");

    tokio::fs::create_dir_all(current_openhuman_dir.join("workspace"))
        .await
        .unwrap();
    tokio::fs::create_dir_all(&sibling_user_dir).await.unwrap();
    tokio::fs::write(&sibling_file, "api_key = 'sibling'\n")
        .await
        .unwrap();

    reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir)
        .await
        .unwrap();

    // The active user is wiped; the sibling account is untouched — this is the
    // regression this fix addresses.
    assert!(!current_openhuman_dir.exists());
    assert!(sibling_user_dir.exists());
    assert!(sibling_file.exists());
}

#[tokio::test]
async fn reset_local_data_tolerates_absent_paths() {
    let temp = tempdir().unwrap();
    let default_openhuman_dir = temp.path().join("default-openhuman");
    let current_openhuman_dir = default_openhuman_dir.join("users").join("active-user");
    tokio::fs::create_dir_all(&default_openhuman_dir)
        .await
        .unwrap();

    // No current user dir, no markers — a fresh / already-cleared install.
    let outcome = reset_local_data_for_paths(&current_openhuman_dir, &default_openhuman_dir)
        .await
        .unwrap();

    assert!(default_openhuman_dir.exists());
    assert!(outcome
        .value
        .get("removed_paths")
        .and_then(|value| value.as_array())
        .is_some_and(|paths| paths.is_empty()));
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
async fn apply_memory_sync_settings_stores_interval_and_view() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    // Pick the 4h preset.
    let patch = MemorySyncSettingsPatch {
        sync_interval_secs: Some(14_400),
    };
    let outcome = apply_memory_sync_settings(&mut cfg, patch)
        .await
        .expect("apply");
    assert_eq!(cfg.memory_sync_interval_secs, Some(14_400));
    assert_eq!(outcome.value["sync_interval_secs"], 14_400);
    assert_eq!(outcome.value["selected_secs"], 14_400);
    assert_eq!(outcome.value["is_manual"], false);
    assert_eq!(outcome.value["is_default"], false);
}

#[tokio::test]
async fn apply_memory_sync_settings_manual_only() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let patch = MemorySyncSettingsPatch {
        sync_interval_secs: Some(0),
    };
    let outcome = apply_memory_sync_settings(&mut cfg, patch)
        .await
        .expect("apply");
    assert_eq!(cfg.memory_sync_interval_secs, Some(0));
    assert_eq!(outcome.value["is_manual"], true);
    assert_eq!(outcome.value["sync_interval_secs"], 0);
}

#[tokio::test]
async fn apply_memory_sync_settings_reset_to_default() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.memory_sync_interval_secs = Some(43_200);

    // Omitted field → None → reset to default.
    let patch = MemorySyncSettingsPatch {
        sync_interval_secs: None,
    };
    let outcome = apply_memory_sync_settings(&mut cfg, patch)
        .await
        .expect("apply");
    assert_eq!(cfg.memory_sync_interval_secs, None);
    assert_eq!(outcome.value["is_default"], true);
    assert!(outcome.value["sync_interval_secs"].is_null());
    // The UI still gets a concrete cadence to highlight (the 24h default).
    assert_eq!(
        outcome.value["selected_secs"],
        crate::openhuman::config::DEFAULT_MEMORY_SYNC_INTERVAL_SECS
    );
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
async fn apply_model_settings_replaces_model_registry_when_some_and_keeps_when_none() {
    // Per-model vision registry follows Some=replace / None=keep / empty=clear —
    // this persists the "Supports vision" flag set in Settings → Advanced LLM.
    use crate::openhuman::config::schema::ModelRegistryEntry;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let set = ModelSettingsPatch {
        model_registry: Some(vec![ModelRegistryEntry {
            id: "my-llava".into(),
            provider: "openai".into(),
            cost_per_1m_output: 0.0,
            vision: true,
            ..Default::default()
        }]),
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, set).await.expect("set");
    assert_eq!(cfg.model_registry.len(), 1);
    assert!(cfg
        .model_registry
        .iter()
        .any(|e| e.id == "my-llava" && e.vision));

    // None — leave registry alone.
    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            api_url: Some("https://x.test/v1".into()),
            ..Default::default()
        },
    )
    .await
    .expect("touch");
    assert_eq!(cfg.model_registry.len(), 1);

    // Empty vec — clear.
    let _ = apply_model_settings(
        &mut cfg,
        ModelSettingsPatch {
            model_registry: Some(vec![]),
            ..Default::default()
        },
    )
    .await
    .expect("clear");
    assert!(cfg.model_registry.is_empty());
}

#[tokio::test]
async fn apply_model_settings_trims_model_registry_ids() {
    // `model_vision_enabled` matches the resolved id exactly, so persisted ids
    // must be trimmed or stray whitespace would silently disable vision.
    use crate::openhuman::config::schema::ModelRegistryEntry;
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let set = ModelSettingsPatch {
        model_registry: Some(vec![ModelRegistryEntry {
            id: "  spaced-model  ".into(),
            provider: "openai".into(),
            cost_per_1m_output: 0.0,
            vision: true,
            ..Default::default()
        }]),
        ..Default::default()
    };
    let _ = apply_model_settings(&mut cfg, set).await.expect("set");
    assert_eq!(cfg.model_registry.len(), 1);
    assert_eq!(cfg.model_registry[0].id, "spaced-model");
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
        base_url: Some(Some(" http://localhost:1234/v1/ ".into())),
        model_id: Some(" local-default ".into()),
        chat_model_id: Some(" local-chat ".into()),
        usage_embeddings: Some(true),
        usage_heartbeat: Some(true),
        usage_learning_reflection: Some(false),
        usage_subconscious: Some(true),
        api_key: None,
    };

    let outcome = apply_local_ai_settings(&mut cfg, patch)
        .await
        .expect("apply local ai");

    assert!(cfg.local_ai.runtime_enabled);
    assert!(cfg.local_ai.opt_in_confirmed);
    assert_eq!(cfg.local_ai.provider, "lm_studio");
    assert_eq!(
        cfg.local_ai.base_url.as_deref(),
        Some("http://localhost:1234/v1")
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
        base_url: Some(Some("   ".into())),
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
async fn apply_local_ai_settings_normalizes_ollama_unspecified_host_and_allows_null_clear() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    apply_local_ai_settings(
        &mut cfg,
        LocalAiSettingsPatch {
            provider: Some("ollama".into()),
            base_url: Some(Some("http://0.0.0.0:11434/api/tags".into())),
            ..LocalAiSettingsPatch::default()
        },
    )
    .await
    .expect("apply ollama base url");

    assert_eq!(
        cfg.local_ai.base_url.as_deref(),
        Some("http://localhost:11434")
    );

    apply_local_ai_settings(
        &mut cfg,
        LocalAiSettingsPatch {
            base_url: Some(None),
            ..LocalAiSettingsPatch::default()
        },
    )
    .await
    .expect("clear ollama base url");

    assert!(cfg.local_ai.base_url.is_none());
}

#[tokio::test]
async fn apply_local_ai_settings_persists_api_key() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.local_ai.api_key = None;

    // Non-empty key is stored.
    let patch = LocalAiSettingsPatch {
        runtime_enabled: Some(true),
        opt_in_confirmed: Some(true),
        provider: Some("omlx".into()),
        base_url: Some(Some("http://localhost:8080/v1".into())),
        api_key: Some("sk-omlx-1".into()),
        ..LocalAiSettingsPatch::default()
    };
    apply_local_ai_settings(&mut cfg, patch)
        .await
        .expect("apply omlx api key");
    assert_eq!(cfg.local_ai.api_key.as_deref(), Some("sk-omlx-1"));

    // Whitespace-only key clears to None.
    let patch_clear = LocalAiSettingsPatch {
        api_key: Some("   ".into()),
        ..LocalAiSettingsPatch::default()
    };
    apply_local_ai_settings(&mut cfg, patch_clear)
        .await
        .expect("clear api key");
    assert!(cfg.local_ai.api_key.is_none());
}

#[tokio::test]
async fn apply_local_ai_settings_omlx_keeps_provider_and_v1_suffix() {
    // Regression: omlx must NOT collapse to ollama (normalize_provider) and its
    // `/v1` suffix must survive (no validate_ollama_url path-strip).
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    apply_local_ai_settings(
        &mut cfg,
        LocalAiSettingsPatch {
            runtime_enabled: Some(true),
            opt_in_confirmed: Some(true),
            provider: Some("omlx".into()),
            base_url: Some(Some("http://localhost:8000/v1".into())),
            api_key: Some("sk-omlx-1".into()),
            ..LocalAiSettingsPatch::default()
        },
    )
    .await
    .expect("apply omlx");

    assert_eq!(cfg.local_ai.provider, "omlx");
    assert_eq!(
        cfg.local_ai.base_url.as_deref(),
        Some("http://localhost:8000/v1")
    );
    assert_eq!(cfg.local_ai.api_key.as_deref(), Some("sk-omlx-1"));
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
            ..Default::default()
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
            ..Default::default()
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
            ..Default::default()
        },
    )
    .await
    .expect("apply noop");
    assert_eq!(prior, cfg.meet.auto_orchestrator_handoff);
}

#[tokio::test]
async fn apply_meet_settings_updates_all_meeting_assistant_fields() {
    use crate::openhuman::config::{AutoJoinPolicy, AutoSummarizePolicy};
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    // Defaults (issue #3511).
    assert_eq!(cfg.meet.auto_join_policy, AutoJoinPolicy::AskEachTime);
    assert_eq!(cfg.meet.auto_summarize_policy, AutoSummarizePolicy::Ask);
    assert!(cfg.meet.listen_only_default);
    assert!(!cfg.meet.ingest_backend_transcripts);

    let _ = apply_meet_settings(
        &mut cfg,
        MeetSettingsPatch {
            auto_join_policy: Some(AutoJoinPolicy::Always),
            auto_summarize_policy: Some(AutoSummarizePolicy::Never),
            listen_only_default: Some(false),
            ingest_backend_transcripts: Some(true),
            ..Default::default()
        },
    )
    .await
    .expect("apply all fields");
    assert_eq!(cfg.meet.auto_join_policy, AutoJoinPolicy::Always);
    assert_eq!(cfg.meet.auto_summarize_policy, AutoSummarizePolicy::Never);
    assert!(!cfg.meet.listen_only_default);
    assert!(cfg.meet.ingest_backend_transcripts);

    // No-op patch must leave the prior values untouched.
    let _ = apply_meet_settings(&mut cfg, MeetSettingsPatch::default())
        .await
        .expect("apply noop");
    assert_eq!(cfg.meet.auto_join_policy, AutoJoinPolicy::Always);
    assert_eq!(cfg.meet.auto_summarize_policy, AutoSummarizePolicy::Never);
    assert!(!cfg.meet.listen_only_default);
    assert!(cfg.meet.ingest_backend_transcripts);
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
        always_on_enabled: None,
        wake_word: None,
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
        always_on_enabled: Some(true),
        wake_word: Some("Hey Tiny".to_string()),
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
        vision_provider: Some(" provider-vision ".into()),
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
    assert_eq!(cfg.vision_provider.as_deref(), Some("provider-vision"));

    let clear = ModelSettingsPatch {
        inference_url: Some("   ".into()),
        primary_cloud: Some("".into()),
        reasoning_provider: Some(" ".into()),
        agentic_provider: Some(" ".into()),
        coding_provider: Some(" ".into()),
        vision_provider: Some(" ".into()),
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
    assert!(cfg.vision_provider.is_none());
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

// ── agent settings (action/tool timeout, issue #3100) ───────────────────────

#[tokio::test]
async fn apply_agent_settings_updates_timeout_and_persists_snapshot() {
    // ENV_LOCK: `set_tool_timeout_secs` reads OPENHUMAN_TOOL_TIMEOUT_SECS and
    // mutates the process-global timeout; serialize against other env-touching
    // tests and ensure no operator override is masking the config value.
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_TOOL_TIMEOUT_SECS");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let outcome = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(300),
        },
    )
    .await
    .expect("apply agent settings");

    assert_eq!(cfg.agent.agent_timeout_secs, 300);
    assert_eq!(
        outcome.value["config"]["agent"]["agent_timeout_secs"],
        serde_json::json!(300)
    );
    assert!(outcome
        .logs
        .iter()
        .any(|l| l.contains("agent settings saved to")));
    // With no env override, the live runtime now reflects the saved value.
    assert_eq!(
        crate::openhuman::tool_timeout::tool_execution_timeout_secs(),
        300
    );
}

#[tokio::test]
async fn apply_agent_settings_rejects_out_of_range_timeout() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let original = cfg.agent.agent_timeout_secs;

    // Zero would disable the timeout — rejected.
    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(0),
        },
    )
    .await
    .expect_err("zero timeout should be rejected");
    assert!(err.contains("between"), "unexpected error: {err}");

    // Above the 3600s ceiling — rejected.
    let err = apply_agent_settings(
        &mut cfg,
        AgentSettingsPatch {
            agent_timeout_secs: Some(99_999),
        },
    )
    .await
    .expect_err("over-max timeout should be rejected");
    assert!(err.contains("between"), "unexpected error: {err}");

    // The config value is untouched after a rejected update.
    assert_eq!(cfg.agent.agent_timeout_secs, original);
}

#[tokio::test]
async fn apply_agent_settings_none_leaves_timeout_unchanged() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.agent.agent_timeout_secs = 250;

    apply_agent_settings(&mut cfg, AgentSettingsPatch::default())
        .await
        .expect("apply no-op agent settings");

    assert_eq!(cfg.agent.agent_timeout_secs, 250);
}

// ── apply_agent_paths_settings (action_dir editable, issue #3240) ──────────────

#[tokio::test]
async fn apply_agent_paths_valid_abs_path_persists_override_and_recomputes() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Ensure no env override is interfering.
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let new_dir = tmp.path().join("agent-projects");
    std::fs::create_dir_all(&new_dir).unwrap();

    let outcome = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(new_dir.to_string_lossy().to_string()),
        },
    )
    .await
    .expect("apply agent paths");

    assert_eq!(cfg.action_dir_override.as_deref(), Some(new_dir.as_path()));
    assert_eq!(cfg.action_dir, new_dir);
    assert_eq!(
        outcome.value["action_dir"],
        serde_json::json!(new_dir.display().to_string())
    );
    assert_eq!(
        outcome.value["action_dir_source"],
        serde_json::json!("override")
    );
}

#[tokio::test]
async fn apply_agent_paths_rejects_relative_path() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);

    let err = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some("relative/projects".into()),
        },
    )
    .await
    .expect_err("relative path must be rejected");

    assert!(err.contains("absolute"), "unexpected error: {err}");
    assert!(cfg.action_dir_override.is_none());
}

#[tokio::test]
async fn apply_agent_paths_rejects_action_dir_equal_to_workspace() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let workspace = cfg.workspace_dir.clone();

    let err = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(workspace.to_string_lossy().to_string()),
        },
    )
    .await
    .expect_err("action_dir == workspace_dir must be rejected");

    assert!(err.contains("workspace"), "unexpected error: {err}");
    assert!(cfg.action_dir_override.is_none());
}

#[tokio::test]
async fn apply_agent_paths_empty_input_clears_override() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    // Start with an override in place.
    let prior = tmp.path().join("prior-projects");
    std::fs::create_dir_all(&prior).unwrap();
    cfg.action_dir_override = Some(prior);

    let outcome = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some("   ".into()),
        },
    )
    .await
    .expect("clear override");

    assert!(cfg.action_dir_override.is_none());
    // Reverts to the default projects dir.
    assert_eq!(
        cfg.action_dir,
        crate::openhuman::config::default_projects_dir()
    );
    assert_eq!(
        outcome.value["action_dir_source"],
        serde_json::json!("default")
    );
}

#[tokio::test]
async fn apply_agent_paths_auto_creates_missing_directory() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let missing = tmp.path().join("not-yet").join("created");
    assert!(!missing.exists());

    apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(missing.to_string_lossy().to_string()),
        },
    )
    .await
    .expect("auto-create action dir");

    assert!(missing.is_dir(), "missing action_dir must be auto-created");
    assert_eq!(cfg.action_dir, missing);
}

#[tokio::test]
async fn apply_agent_paths_rejects_existing_file() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let file = tmp.path().join("a-file.txt");
    std::fs::write(&file, b"not a dir").unwrap();

    let err = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(file.to_string_lossy().to_string()),
        },
    )
    .await
    .expect_err("a file path must be rejected");

    assert!(err.contains("directory"), "unexpected error: {err}");
    assert!(cfg.action_dir_override.is_none());
}

#[tokio::test]
async fn apply_agent_paths_env_set_reports_source_env() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    let env_dir = tmp.path().join("env-pinned");
    std::fs::create_dir_all(&env_dir).unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_ACTION_DIR", &env_dir);
    }

    let mut cfg = tmp_config(&tmp);
    // Even when the user sets an override, the env var wins for the effective
    // value and the reported source.
    let user_dir = tmp.path().join("user-choice");
    std::fs::create_dir_all(&user_dir).unwrap();

    let outcome = apply_agent_paths_settings(
        &mut cfg,
        AgentPathsPatch {
            action_dir: Some(user_dir.to_string_lossy().to_string()),
        },
    )
    .await
    .expect("apply with env override present");

    // Override is persisted, but the effective action_dir reflects the env.
    assert_eq!(cfg.action_dir_override.as_deref(), Some(user_dir.as_path()));
    assert_eq!(cfg.action_dir, env_dir);
    assert_eq!(outcome.value["action_dir_source"], serde_json::json!("env"));

    unsafe {
        std::env::remove_var("OPENHUMAN_ACTION_DIR");
    }
}

// --- #3353 regression tests -------------------------------------------------

#[test]
fn expand_tilde_happy_path_uses_home() {
    // `~/OpenHuman/projects` resolves to the home dir joined component-wise.
    let expanded = expand_tilde("~/OpenHuman/projects");
    let expected = dirs::home_dir()
        .expect("home dir resolvable in test env")
        .join("OpenHuman")
        .join("projects");
    assert_eq!(expanded, expected.to_string_lossy());
}

#[test]
fn expand_tilde_without_prefix_is_unchanged() {
    // Absolute paths and a bare `~` (no trailing slash) pass through verbatim.
    assert_eq!(expand_tilde("/abs/path"), "/abs/path");
    assert_eq!(expand_tilde("~"), "~");
    assert_eq!(expand_tilde("relative/path"), "relative/path");
}

#[cfg(windows)]
#[test]
fn expand_tilde_has_no_mixed_separators_on_windows() {
    // The whole point of the component-wise build: the result must be a pure
    // backslash path with no embedded forward slash, so CreateProcessW accepts
    // it as a CWD instead of failing with ERROR_DIRECTORY (os error 267).
    let expanded = expand_tilde("~/OpenHuman/projects");
    assert!(
        !expanded.contains('/'),
        "expected no forward slashes on Windows, got: {expanded}"
    );
}

#[test]
fn redact_home_replaces_home_prefix_and_passes_through_others() {
    let home = dirs::home_dir().expect("home dir resolvable in test env");

    // A path under home is redacted to `~/...` — the username/home prefix is
    // stripped but the diagnostic suffix is preserved.
    let under_home = home.join("OpenHuman").join("projects");
    let redacted = redact_home(&under_home);
    assert!(
        redacted.starts_with('~'),
        "expected a `~`-prefixed path, got: {redacted}"
    );
    assert!(
        !redacted.contains(&*home.to_string_lossy()),
        "redacted path must not contain the raw home dir: {redacted}"
    );
    assert!(
        redacted.contains("OpenHuman"),
        "diagnostic suffix should be preserved: {redacted}"
    );

    // A path outside the home dir is returned unchanged.
    let outside = std::path::Path::new("/var/lib/openhuman/action");
    assert_eq!(redact_home(outside), "/var/lib/openhuman/action");
}

#[tokio::test]
async fn ensure_agent_dirs_creates_missing_action_dir_and_trusted_root() {
    use crate::openhuman::security::TrustedAccess;

    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    // Point the default projects home at the tempdir so the helper doesn't touch
    // the real `~/OpenHuman/projects`.
    let projects_dir = tmp.path().join("projects-home");
    let prev_projects_dir = std::env::var_os("OPENHUMAN_PROJECTS_DIR");
    unsafe {
        std::env::set_var("OPENHUMAN_PROJECTS_DIR", &projects_dir);
    }

    let mut cfg = tmp_config(&tmp);
    let action_dir = tmp.path().join("fresh-action-dir");
    cfg.action_dir = action_dir.clone();
    assert!(!action_dir.exists(), "precondition: action_dir is missing");

    crate::openhuman::config::ensure_agent_dirs(&mut cfg).await;

    // Both the action_dir and the projects home now exist.
    assert!(action_dir.is_dir(), "action_dir should be created");
    assert!(projects_dir.is_dir(), "projects home should be created");

    // The projects home is registered exactly once as a ReadWrite trusted root.
    let projects_path = projects_dir.to_string_lossy().to_string();
    let matching: Vec<_> = cfg
        .autonomy
        .trusted_roots
        .iter()
        .filter(|r| r.path == projects_path)
        .collect();
    assert_eq!(matching.len(), 1, "trusted root registered exactly once");
    assert!(matches!(matching[0].access, TrustedAccess::ReadWrite));

    // Idempotent: a second call neither errors nor duplicates the trusted root.
    crate::openhuman::config::ensure_agent_dirs(&mut cfg).await;
    let count = cfg
        .autonomy
        .trusted_roots
        .iter()
        .filter(|r| r.path == projects_path)
        .count();
    assert_eq!(count, 1, "second call must not duplicate the trusted root");

    // Restore the prior env state so later tests observe the real environment.
    unsafe {
        match prev_projects_dir {
            Some(v) => std::env::set_var("OPENHUMAN_PROJECTS_DIR", v),
            None => std::env::remove_var("OPENHUMAN_PROJECTS_DIR"),
        }
    }
}

#[test]
fn ensure_usable_cwd_creates_missing_dir() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().join("not-yet-here");
    assert!(!dir.exists());
    crate::openhuman::config::ensure_usable_cwd(&dir).expect("missing dir is created");
    assert!(dir.is_dir());
}

#[test]
fn ensure_usable_cwd_rejects_a_file_with_descriptive_error() {
    let tmp = tempdir().unwrap();
    let file = tmp.path().join("a-file");
    std::fs::write(&file, b"x").unwrap();
    let err = crate::openhuman::config::ensure_usable_cwd(&file)
        .expect_err("an existing file is not a usable working directory");
    let msg = err.to_string();
    assert!(msg.contains("not a directory"), "unexpected error: {msg}");
    // The error names the offending path so the message is actionable.
    assert!(
        msg.contains(&file.to_string_lossy().to_string()),
        "error should name the path: {msg}"
    );
}

#[test]
fn ensure_usable_cwd_errors_when_uncreatable() {
    // A directory whose parent is an existing *file* can't be created; the
    // helper must surface the descriptive, path-naming error rather than panic.
    let tmp = tempdir().unwrap();
    let parent_file = tmp.path().join("parent-file");
    std::fs::write(&parent_file, b"x").unwrap();
    let target = parent_file.join("child");
    let err = crate::openhuman::config::ensure_usable_cwd(&target)
        .expect_err("cannot create a dir under a file");
    let msg = err.to_string();
    assert!(
        msg.contains("could not be created"),
        "unexpected error: {msg}"
    );
}
