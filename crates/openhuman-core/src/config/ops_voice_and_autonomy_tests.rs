use super::*;

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
        stt_engine: None,
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
        stt_engine: Some("elevenlabs".into()),
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
    assert_eq!(
        outcome.value["config"]["voice_server"]["stt_engine"]
            .as_str()
            .unwrap_or_default(),
        "elevenlabs",
        "the engine picker must persist through the config update RPC"
    );
    unsafe {
        std::env::remove_var("OPENHUMAN_WORKSPACE");
    }
}

/// An engine name the core does not know must fail loudly. Defaulting to the
/// backend proxy would silently transcribe (and bill) somewhere the caller did
/// not ask for.
#[tokio::test]
async fn load_and_apply_voice_server_settings_rejects_unknown_stt_engine() {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let tmp = tempdir().unwrap();
    unsafe {
        std::env::set_var("OPENHUMAN_WORKSPACE", tmp.path());
    }
    let patch = VoiceServerSettingsPatch {
        auto_start: None,
        hotkey: None,
        activation_mode: None,
        skip_cleanup: None,
        min_duration_secs: None,
        silence_threshold: None,
        custom_dictionary: None,
        always_on_enabled: None,
        wake_word: None,
        // The removed local engine is the case that matters: an old client
        // could still send it.
        stt_engine: Some("whisper".into()),
    };
    let err = load_and_apply_voice_server_settings(patch)
        .await
        .unwrap_err();
    assert!(err.contains("invalid stt_engine"), "got: {err}");
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

#[test]
fn resolve_api_url_keeps_inference_overrides_away_from_backend_credentials() {
    let mut config = Config::default();
    let expected_backend = crate::api::config::effective_backend_api_url(&None);

    for inference_url in ["http://localhost:11434/v1", "https://openrouter.ai/api/v1"] {
        config.api_url = Some(inference_url.to_string());
        let resolved = resolve_backend_api_url(&config);
        assert_ne!(resolved, inference_url);
        assert_eq!(resolved, expected_backend);
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
