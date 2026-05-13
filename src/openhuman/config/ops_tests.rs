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
fn set_browser_allow_all_toggles_env_var() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let before = std::env::var("OPENHUMAN_BROWSER_ALLOW_ALL").ok();

    let _ = set_browser_allow_all(true);
    assert!(env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"));

    let _ = set_browser_allow_all(false);
    assert!(!env_flag_enabled("OPENHUMAN_BROWSER_ALLOW_ALL"));

    unsafe {
        match before {
            Some(v) => std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", v),
            None => std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL"),
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
        api_key: None,
        default_model: Some("gpt-4o".into()),
        default_temperature: Some(0.25),
        model_routes: None,
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
async fn apply_model_settings_stores_api_key_and_clears_when_empty() {
    // #1342: custom OpenAI-compatible providers — api_key must round-trip
    // through `apply_model_settings` and clear when an empty string is sent.
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    let set = ModelSettingsPatch {
        api_url: Some("https://llm.example.test/v1".into()),
        api_key: Some("  sk-test-1234  ".into()),
        default_model: Some("gpt-4o-mini".into()),
        default_temperature: None,
        model_routes: None,
    };
    let _ = apply_model_settings(&mut cfg, set).await.expect("set");
    assert_eq!(cfg.api_key.as_deref(), Some("sk-test-1234"));

    let clear = ModelSettingsPatch {
        api_url: None,
        api_key: Some("".into()),
        default_model: None,
        default_temperature: None,
        model_routes: None,
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
    };
    let _ = apply_model_settings(&mut cfg, set_routes)
        .await
        .expect("set");
    assert_eq!(cfg.model_routes.len(), 2);
    assert_eq!(cfg.model_routes[0].hint, "reasoning");

    // None — leave routes alone.
    let touch_other = ModelSettingsPatch {
        api_url: Some("https://x.test/v1".into()),
        api_key: None,
        default_model: None,
        default_temperature: None,
        model_routes: None,
    };
    let _ = apply_model_settings(&mut cfg, touch_other)
        .await
        .expect("touch");
    assert_eq!(cfg.model_routes.len(), 2);
    assert_eq!(cfg.api_url.as_deref(), Some("https://x.test/v1"));

    // Empty vec — clear.
    let clear_routes = ModelSettingsPatch {
        api_url: None,
        api_key: None,
        default_model: None,
        default_temperature: None,
        model_routes: Some(vec![]),
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
        api_key: None,
        default_model: Some("".into()),
        default_temperature: None,
        model_routes: None,
    };
    let _ = apply_model_settings(&mut cfg, patch).await.expect("apply");
    assert!(cfg.api_url.is_none());
    assert!(cfg.default_model.is_none());
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
