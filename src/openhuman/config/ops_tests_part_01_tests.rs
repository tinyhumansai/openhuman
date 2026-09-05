use super::*;

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

#[test]
fn snapshot_config_json_masks_youpet_service_token() {
    let tmp = tempdir().unwrap();
    let mut cfg = Config::default();
    cfg.workspace_dir = tmp.path().join("workspace");
    cfg.config_path = tmp.path().join("config.toml");
    cfg.youpet.service_token = Some("youpet-secret-token".into());

    let snap = snapshot_config_json(&cfg).expect("snapshot should succeed");
    let youpet = &snap["config"]["youpet"];

    assert_eq!(youpet["service_token_set"], true);
    assert!(youpet.get("service_token").is_none());
    assert!(!snap.to_string().contains("youpet-secret-token"));
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

/// #5324 (CodeRabbit): the failed-job un-park must be scoped to an embedder
/// change. Saving an unrelated model setting (temperature, chat model, …)
/// through this shared path must leave terminally-`failed` embedding jobs
/// parked, not restart them into the same external failure. Switching the
/// embeddings provider is what un-parks them.
///
/// The gate is the host's rule, so that is what this pins: whether the driver
/// is asked at all, and what the outcome line reports. Whether the ask then
/// moves a row from `failed` to `ready` is the driver's, and is pinned in the
/// driver's own conformance suite against a real queue.
#[tokio::test]
async fn apply_model_settings_requeues_failed_jobs_only_on_embedder_change() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let driver = crate::openhuman::memory::binding::install_retrying_driver_for_test(&cfg, 1);

    // Unrelated save (temperature only) — the driver must not be asked.
    let unrelated = ModelSettingsPatch {
        default_temperature: Some(0.5),
        ..Default::default()
    };
    let outcome = apply_model_settings(&mut cfg, unrelated)
        .await
        .expect("apply");
    assert_eq!(
        driver.retry_calls(),
        0,
        "an unrelated model save must not ask the driver to un-park anything"
    );
    assert!(
        outcome.logs.iter().any(|m| m.contains("requeued_failed=0")),
        "messages: {:?}",
        outcome.logs
    );

    // Now change the embeddings provider — this is the remediation, so the
    // driver is asked and its count is reported.
    let switch = ModelSettingsPatch {
        embeddings_provider: Some("ollama:bge-m3".into()),
        ..Default::default()
    };
    let outcome = apply_model_settings(&mut cfg, switch).await.expect("apply");
    assert_eq!(
        driver.retry_calls(),
        1,
        "switching the embeddings provider must ask the driver to un-park"
    );
    assert!(
        outcome.logs.iter().any(|m| m.contains("requeued_failed=1")),
        "the driver's count reaches the outcome line: {:?}",
        outcome.logs
    );
}

/// #5324 (CodeRabbit): mirror of the model-settings gate for the memory path.
/// A `memory_window` / `auto_save` / `backend` save shares this function but
/// does not remediate the embedder, so failed jobs must stay parked; changing
/// the embedding provider un-parks them.
#[tokio::test]
async fn apply_memory_settings_requeues_failed_jobs_only_on_embedder_change() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let driver = crate::openhuman::memory::binding::install_retrying_driver_for_test(&cfg, 1);

    // Unrelated save (memory window preset only) — the driver is not asked.
    let unrelated = MemorySettingsPatch {
        memory_window: Some("balanced".into()),
        ..Default::default()
    };
    let outcome = apply_memory_settings(&mut cfg, unrelated)
        .await
        .expect("apply");
    assert_eq!(
        driver.retry_calls(),
        0,
        "a memory-window save must not ask the driver to un-park anything"
    );
    assert!(outcome.logs.iter().any(|m| m.contains("requeued_failed=0")));

    // Change the embedding provider — the driver is asked.
    let switch = MemorySettingsPatch {
        embedding_provider: Some("ollama".into()),
        ..Default::default()
    };
    let outcome = apply_memory_settings(&mut cfg, switch)
        .await
        .expect("apply");
    assert_eq!(
        driver.retry_calls(),
        1,
        "switching the embedding provider must ask the driver to un-park"
    );
    assert!(outcome.logs.iter().any(|m| m.contains("requeued_failed=1")));
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

    assert!(err.contains("disabled/managed/parallel/brave/querit/exa"));
}
