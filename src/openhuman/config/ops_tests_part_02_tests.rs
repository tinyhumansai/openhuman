use super::*;

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
            backend: None,
        },
    )
    .await
    .expect("apply");
    assert!(cfg.browser.enabled);
}

#[tokio::test]
async fn apply_browser_settings_updates_backend() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.browser.backend = "agent_browser".into();

    apply_browser_settings(
        &mut cfg,
        BrowserSettingsPatch {
            enabled: None,
            backend: Some("playwright".into()),
        },
    )
    .await
    .expect("apply");

    assert_eq!(cfg.browser.backend, "playwright");
}

#[tokio::test]
async fn apply_browser_settings_rejects_unknown_backend() {
    let tmp = tempdir().unwrap();
    let mut cfg = tmp_config(&tmp);
    cfg.browser.enabled = false;
    cfg.browser.backend = "agent_browser".into();

    let err = apply_browser_settings(
        &mut cfg,
        BrowserSettingsPatch {
            enabled: Some(true),
            backend: Some("netscape".into()),
        },
    )
    .await
    .expect_err("unknown backend should fail");

    assert!(err.contains("Unsupported browser backend"));
    assert!(!cfg.browser.enabled);
    assert_eq!(cfg.browser.backend, "agent_browser");
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
