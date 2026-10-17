use super::effective_session_config;
use crate::config::Config;
use crate::inference::provider::provider_for_role;

fn with_huggingface_route(config: &mut Config) {
    config
        .cloud_providers
        .push(crate::config::schema::CloudProviderCreds {
            id: "huggingface-test".to_string(),
            slug: "huggingface".to_string(),
            label: "Hugging Face test route".to_string(),
            endpoint: "https://huggingface.co".to_string(),
            ..Default::default()
        });
}

#[test]
fn picker_routes_follow_provider_and_model_across_turns() {
    let mut original = Config::default();
    original.chat_provider = Some("openai:gpt-4o".to_string());
    original.reasoning_provider = Some("anthropic:claude-sonnet".to_string());
    original.coding_provider = Some("openai:o3-mini".to_string());
    with_huggingface_route(&mut original);
    let original_snapshot = serde_json::to_value(&original).expect("config serializes");

    let mut selected = effective_session_config(
        &original,
        Some("openrouter/author/managed-model:free"),
        None,
    );
    assert_eq!(
        provider_for_role("chat", &selected),
        "openhuman",
        "managed catalog selection replaces a prior BYOK route"
    );
    assert_eq!(
        selected.default_model.as_deref(),
        Some("openrouter/author/managed-model:free")
    );

    selected = effective_session_config(&selected, Some("ollama:gpt-oss:120b-cloud"), None);
    assert_eq!(
        provider_for_role("chat", &selected),
        "ollama:gpt-oss:120b-cloud",
        "local picker selection retains the model suffix after the provider colon"
    );
    assert_eq!(
        selected.default_model.as_deref(),
        Some("ollama:gpt-oss:120b-cloud")
    );

    selected = effective_session_config(&selected, Some("ollama:qwen3:4b-instruct"), None);
    assert_eq!(
        provider_for_role("chat", &selected),
        "ollama:qwen3:4b-instruct"
    );

    selected = effective_session_config(&selected, Some("huggingface:org/model"), None);
    assert_eq!(
        provider_for_role("chat", &selected),
        "huggingface:org/model"
    );

    selected = effective_session_config(
        &selected,
        Some("openrouter/author/managed-model:free"),
        None,
    );
    assert_eq!(
        provider_for_role("chat", &selected),
        "openhuman",
        "managed selection clears a previously selected local or cloud route"
    );

    assert_eq!(
        serde_json::to_value(&original).expect("config serializes"),
        original_snapshot,
        "turn selections must not mutate the saved config"
    );
    assert_eq!(
        provider_for_role("reasoning", &selected),
        "anthropic:claude-sonnet",
        "changing the chat picker must not change sibling workload routes"
    );
    assert_eq!(provider_for_role("coding", &selected), "openai:o3-mini");
}

#[test]
fn persisted_managed_default_restores_managed_route_after_restart() {
    let mut persisted = Config::default();
    persisted.default_model = Some("openrouter/author/restored-model:free".to_string());
    persisted.chat_provider = Some("ollama:qwen3:4b-instruct".to_string());

    let effective = effective_session_config(&persisted, None, None);
    assert_eq!(
        effective.default_model.as_deref(),
        Some("openrouter/author/restored-model:free")
    );
    assert_eq!(provider_for_role("chat", &effective), "openhuman");
    assert_eq!(
        persisted.chat_provider.as_deref(),
        Some("ollama:qwen3:4b-instruct"),
        "restoration is turn-local and leaves persisted routing alone"
    );
}

#[test]
fn persisted_local_default_restores_its_route_after_restart() {
    let mut persisted = Config::default();
    persisted.default_model = Some("ollama:qwen3:4b-instruct".to_string());
    persisted.chat_provider = Some("openai:gpt-4o".to_string());

    let effective = effective_session_config(&persisted, None, None);
    assert_eq!(
        effective.default_model.as_deref(),
        Some("ollama:qwen3:4b-instruct")
    );
    assert_eq!(
        provider_for_role("chat", &effective),
        "ollama:qwen3:4b-instruct"
    );
}

#[test]
fn hints_and_legacy_model_ids_keep_the_configured_route() {
    let mut config = Config::default();
    config.default_model = Some("stored-default".to_string());
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.coding_provider = Some("ollama:qwen3-coder:latest".to_string());
    let original = config.clone();

    let hint = effective_session_config(&config, Some("hint:coding"), None);
    assert_eq!(
        provider_for_role("coding", &hint),
        "ollama:qwen3-coder:latest"
    );
    assert_eq!(provider_for_role("chat", &hint), "openai:gpt-4o");

    let legacy_alias = effective_session_config(&config, Some("coding-v1"), None);
    assert_eq!(
        provider_for_role("coding", &legacy_alias),
        "ollama:qwen3-coder:latest"
    );

    let unqualified = effective_session_config(&config, Some("gpt-4.1-mini"), None);
    assert_eq!(provider_for_role("chat", &unqualified), "openai:gpt-4o");
    assert_eq!(unqualified.default_model.as_deref(), Some("gpt-4.1-mini"));

    for selection in [
        "claude-code:claude-sonnet-4-20250514",
        "claude_agent_sdk:claude-sonnet-4-20250514",
    ] {
        let built_in = effective_session_config(&config, Some(selection), None);
        assert_eq!(
            provider_for_role("chat", &built_in),
            selection,
            "built-in provider route {selection} is recognized without a cloud provider entry"
        );
    }

    assert_eq!(config.default_model, original.default_model);
    assert_eq!(config.chat_provider, original.chat_provider);
    assert_eq!(config.coding_provider, original.coding_provider);
}

#[test]
fn explicit_picker_override_wins_over_a_different_persisted_default() {
    let mut persisted = Config::default();
    persisted.default_model = Some("openrouter/author/old-model:free".to_string());
    persisted.chat_provider = Some("openhuman".to_string());

    let effective = effective_session_config(&persisted, Some("ollama:gpt-oss:120b-cloud"), None);
    assert_eq!(
        effective.default_model.as_deref(),
        Some("ollama:gpt-oss:120b-cloud")
    );
    assert_eq!(
        provider_for_role("chat", &effective),
        "ollama:gpt-oss:120b-cloud"
    );
}

#[test]
fn session_fingerprint_records_the_effective_picker_route() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:qwen3:4b-instruct".to_string());

    let managed = super::build_session_fingerprint(
        &config,
        Some("openrouter/author/model:free".to_string()),
        None,
        "orchestrator".to_string(),
        "chat",
    );
    let local = super::build_session_fingerprint(
        &config,
        Some("ollama:gpt-oss:120b-cloud".to_string()),
        None,
        "orchestrator".to_string(),
        "chat",
    );

    assert_eq!(managed.provider_binding, "openhuman");
    assert_eq!(local.provider_binding, "ollama:gpt-oss:120b-cloud");
    assert_ne!(managed.provider_binding, local.provider_binding);
}

#[test]
fn unknown_qualified_models_keep_the_existing_route() {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());

    let effective = effective_session_config(&config, Some("unknown-provider:some-model"), None);
    assert_eq!(
        provider_for_role("chat", &effective),
        "unknown-provider:some-model",
        "an unrecognized provider slug must remain invalid instead of falling back"
    );
    assert_eq!(
        effective.default_model.as_deref(),
        Some("unknown-provider:some-model"),
        "unknown legacy model values still pass through as the model override"
    );
}

#[test]
fn selected_managed_route_constructs_with_the_picker_model() {
    // Wait out any test that has a mock chat model installed process-wide.
    let _serial = crate::inference::provider::factory::test_provider_override::exclusive();
    let config = Config::default();
    let selection = "openrouter/author/picked-model:free";
    let effective = effective_session_config(&config, Some(selection), None);
    let (_, resolved_model) =
        crate::inference::provider::factory::create_chat_model_with_model_id_in(
            &crate::core::runtime::DispatchCtx::ambient(),
            "chat",
            &effective,
            effective.default_temperature,
        )
        .expect("managed picker selection should construct without network access");
    assert_eq!(resolved_model, selection);
}

#[test]
fn selected_ollama_route_constructs_the_selected_model_without_network() {
    // Wait out any test that has a mock chat model installed process-wide.
    let _serial = crate::inference::provider::factory::test_provider_override::exclusive();
    let mut config = Config::default();
    config.chat_provider = Some("openhuman".to_string());
    config.local_ai.base_url = Some("http://127.0.0.1:1".to_string());

    for model in ["gpt-oss:120b-cloud", "qwen3:4b-instruct"] {
        let selection = format!("ollama:{model}");
        let effective = effective_session_config(&config, Some(&selection), None);
        let (_, resolved_model) =
            crate::inference::provider::factory::create_chat_model_with_model_id_in(
                &crate::core::runtime::DispatchCtx::ambient(),
                "chat",
                &effective,
                effective.default_temperature,
            )
            .expect("local model construction requires no provider request");
        assert_eq!(resolved_model, model);
    }
}

#[test]
fn empty_picker_values_preserve_persisted_selection_and_temperature() {
    let mut config = Config::default();
    config.default_model = Some("ollama:qwen3:4b-instruct".to_string());
    config.chat_provider = Some("openhuman".to_string());
    config.default_temperature = 0.42;
    let snapshot = serde_json::to_value(&config).unwrap();

    for selection in [None, Some(""), Some("  \t ")] {
        let effective = effective_session_config(&config, selection, None);
        assert_eq!(effective.default_model, config.default_model);
        assert_eq!(
            provider_for_role("chat", &effective),
            "ollama:qwen3:4b-instruct"
        );
        assert_eq!(effective.default_temperature, 0.42);
    }
    assert_eq!(serde_json::to_value(&config).unwrap(), snapshot);
}

#[test]
fn temperature_override_is_turn_local_and_changes_the_cache_fingerprint() {
    let mut config = Config::default();
    config.default_temperature = 0.42;
    config.chat_provider = Some("openhuman".to_string());
    let snapshot = serde_json::to_value(&config).unwrap();
    let selection = Some("  ollama:qwen3:4b-instruct  ");
    let effective = effective_session_config(&config, selection, Some(0.0));
    assert_eq!(effective.default_temperature, 0.0);
    assert_eq!(
        effective.default_model.as_deref(),
        Some("ollama:qwen3:4b-instruct")
    );
    assert_eq!(
        provider_for_role("chat", &effective),
        "ollama:qwen3:4b-instruct"
    );

    let fingerprint = |temperature| {
        super::build_session_fingerprint(
            &config,
            Some("ollama:qwen3:4b-instruct".to_string()),
            Some(temperature),
            "orchestrator".to_string(),
            "chat",
        )
    };
    let cold = fingerprint(0.0);
    let warm = fingerprint(0.7);
    assert_eq!(cold.temperature, Some(0.0));
    assert_eq!(warm.temperature, Some(0.7));
    assert_eq!(cold.provider_binding, warm.provider_binding);
    assert_ne!(cold, warm);
    assert_eq!(serde_json::to_value(&config).unwrap(), snapshot);
}

/// Settings changes must invalidate a warm managed chat even when the caller
/// sends no model override and the provider binding stays managed.
#[test]
fn persisted_managed_model_changes_invalidate_the_session_fingerprint() {
    let mut config = Config::default();
    config.chat_provider = Some("openhuman".to_string());
    config.default_model = Some("openrouter/author/first-model:free".to_string());
    let fingerprint = |config: &Config, selection: Option<String>| {
        super::build_session_fingerprint(
            config,
            selection,
            None,
            "orchestrator".to_string(),
            "chat",
        )
    };
    let first = fingerprint(&config, None);
    config.default_model = Some("openrouter/author/second-model:free".to_string());
    let second = fingerprint(&config, None);
    assert_eq!(first.provider_binding, second.provider_binding);
    assert_eq!(first.model_override, second.model_override);
    assert_ne!(
        first, second,
        "the old cached agent must not survive a saved model change"
    );
    assert!(super::fingerprint_diff(&first, &second)
        .iter()
        .any(|field| field.starts_with("effective_model:")));
    config.default_model = None;
    assert_ne!(
        second,
        fingerprint(&config, None),
        "clearing the pin restores the managed fallback"
    );
}

/// A concrete picker choice still wins over a changing saved default; changing
/// that unrelated default must not evict the agent built for the explicit pick.
#[test]
fn explicit_picker_model_keeps_its_fingerprint_when_saved_default_changes() {
    let mut config = Config::default();
    config.chat_provider = Some("openhuman".to_string());
    config.default_model = Some("openrouter/author/first-model:free".to_string());
    let selected = Some("openrouter/author/picked-model:free".to_string());
    let first = super::build_session_fingerprint(
        &config,
        selected.clone(),
        None,
        "orchestrator".to_string(),
        "chat",
    );
    config.default_model = Some("openrouter/author/second-model:free".to_string());
    let second = super::build_session_fingerprint(
        &config,
        selected,
        None,
        "orchestrator".to_string(),
        "chat",
    );
    assert_eq!(first, second);
}

fn configured_huggingface_route() -> Config {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config
        .cloud_providers
        .push(crate::config::schema::CloudProviderCreds {
            id: "p_huggingface".to_string(),
            slug: "huggingface".to_string(),
            label: "Hugging Face".to_string(),
            endpoint: "http://127.0.0.1:1/v1".to_string(),
            auth_style: crate::config::schema::cloud_providers::AuthStyle::None,
            ..Default::default()
        });
    config
}

fn assert_huggingface_model_builds(config: &Config, expected_model: &str) {
    // Wait out any test that has a mock chat model installed process-wide.
    let _serial = crate::inference::provider::factory::test_provider_override::exclusive();
    let (_, resolved_model) =
        crate::inference::provider::factory::create_chat_model_with_model_id_in(
            &crate::core::runtime::DispatchCtx::ambient(),
            "chat",
            config,
            0.0,
        )
        .expect("case-insensitive cloud selection should resolve through its configured slug");
    assert_eq!(resolved_model, expected_model);
}

#[test]
fn mixed_case_cloud_picker_uses_configured_slug_and_preserves_colon_suffix() {
    let _guard = crate::inference::inference_test_guard();
    let config = configured_huggingface_route();
    let original = serde_json::to_value(&config).expect("config serializes");
    let selected = "HuggingFace:org/model:version:variant";
    let effective = effective_session_config(&config, Some(selected), None);

    assert_eq!(
        effective.chat_provider.as_deref(),
        Some("huggingface:org/model:version:variant")
    );
    assert_eq!(effective.default_model.as_deref(), Some(selected));
    assert_huggingface_model_builds(&effective, "org/model:version:variant");
    assert_eq!(
        serde_json::to_value(&config).expect("config serializes"),
        original,
        "picker selection must remain turn-local"
    );
}

#[test]
fn persisted_mixed_case_cloud_default_uses_configured_slug_after_restart() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = configured_huggingface_route();
    let selected = "HUGGINGFACE:org/model:version:variant";
    config.default_model = Some(selected.to_string());
    let original = serde_json::to_value(&config).expect("config serializes");
    let effective = effective_session_config(&config, None, None);

    assert_eq!(
        effective.chat_provider.as_deref(),
        Some("huggingface:org/model:version:variant")
    );
    assert_eq!(effective.default_model.as_deref(), Some(selected));
    assert_huggingface_model_builds(&effective, "org/model:version:variant");
    assert_eq!(
        serde_json::to_value(&config).expect("config serializes"),
        original,
        "restoring a persisted picker default must remain turn-local"
    );
}
