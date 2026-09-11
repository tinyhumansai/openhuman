use super::*;
use crate::openhuman::config::schema::{AuthStyle, CloudProviderCreds};
use std::path::PathBuf;

fn provider() -> CloudProviderCreds {
    CloudProviderCreds {
        id: "p_openai_123".into(),
        slug: "openai-work".into(),
        label: "OpenAI work".into(),
        endpoint: "https://api.openai.com/v1".into(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("gpt-default".into()),
        ..Default::default()
    }
}

#[test]
fn provider_id_and_model_override_all_interactive_workloads() {
    let mut config = Config::default();
    config.cloud_providers.push(provider());
    let managed_default = config.default_model.clone();

    apply_overrides(
        &mut config,
        &CliInferenceOverrides {
            provider: Some("p_openai_123".into()),
            model: Some("gpt-custom".into()),
        },
    );

    for route in [
        &config.chat_provider,
        &config.reasoning_provider,
        &config.agentic_provider,
        &config.coding_provider,
    ] {
        assert_eq!(route.as_deref(), Some("openai-work:gpt-custom"));
    }
    assert_eq!(config.default_model, managed_default);
}

#[test]
fn model_only_preserves_the_active_provider_and_allows_colons_in_model_ids() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:old:tag".into());
    let managed_default = config.default_model.clone();

    apply_overrides(
        &mut config,
        &CliInferenceOverrides {
            provider: None,
            model: Some("qwen3:8b".into()),
        },
    );

    assert_eq!(config.chat_provider.as_deref(), Some("ollama:qwen3:8b"));
    assert_eq!(config.reasoning_provider, config.chat_provider);
    assert_eq!(config.agentic_provider, config.chat_provider);
    assert_eq!(config.coding_provider, config.chat_provider);
    assert_eq!(config.default_model, managed_default);
}

#[test]
fn provider_only_uses_its_configured_default_model() {
    let mut config = Config::default();
    config.cloud_providers.push(provider());

    apply_overrides(
        &mut config,
        &CliInferenceOverrides {
            provider: Some("openai-work".into()),
            model: None,
        },
    );

    assert_eq!(
        config.chat_provider.as_deref(),
        Some("openai-work:gpt-default")
    );
}

#[test]
fn cloud_provider_resolves_to_primary_cloud_or_managed_fallback() {
    let mut config = Config::default();
    config.cloud_providers.push(provider());
    config.primary_cloud = Some("p_openai_123".into());

    assert_eq!(resolve_provider_key(&config, "cloud"), "openai-work");
    assert_eq!(resolve_provider_key(&config, ""), "openai-work");

    config.primary_cloud = Some("missing-provider".into());
    assert_eq!(resolve_provider_key(&config, "cloud"), "openhuman");
}

#[test]
fn local_provider_only_uses_the_configured_chat_model() {
    let mut config = Config::default();
    config.local_ai.chat_model_id = "qwen3:8b".into();
    let managed_default = config.default_model.clone();

    apply_overrides(
        &mut config,
        &CliInferenceOverrides {
            provider: Some("ollama".into()),
            model: None,
        },
    );

    assert_eq!(config.chat_provider.as_deref(), Some("ollama:qwen3:8b"));
    assert_eq!(config.reasoning_provider, config.chat_provider);
    assert_eq!(config.agentic_provider, config.chat_provider);
    assert_eq!(config.coding_provider, config.chat_provider);
    assert_eq!(config.default_model, managed_default);
}

#[tokio::test]
async fn reload_refreshes_the_snapshot_before_an_unrelated_save() {
    let root = tempfile::tempdir().expect("temporary config root");
    let config_path = root.path().join("config.toml");
    let workspace_dir = root.path().join("workspace");

    set_cli_inference_overrides(None, None);
    let mut initial = Config::default();
    initial.config_path = config_path.clone();
    initial.workspace_dir = workspace_dir.clone();
    initial.chat_provider = Some("openai:old".into());
    initial.save().await.expect("save initial config");

    set_cli_inference_overrides(Some("ollama"), Some("qwen3:8b"));
    let mut first = Config::load_from_config_path(&config_path, &workspace_dir)
        .await
        .expect("first load");
    first.chat_provider = Some("anthropic:explicit".into());
    first.save().await.expect("save explicit route change");

    let mut reloaded = Config::load_from_config_path(&config_path, &workspace_dir)
        .await
        .expect("reload after explicit change");
    reloaded.onboarding_completed = true;
    reloaded.save().await.expect("save unrelated change");

    set_cli_inference_overrides(None, None);
    let persisted = Config::load_from_config_path(&config_path, &workspace_dir)
        .await
        .expect("load persisted config without overrides");

    assert_eq!(
        persisted.chat_provider.as_deref(),
        Some("anthropic:explicit")
    );
    assert!(persisted.onboarding_completed);
}

#[test]
fn persistence_restore_keeps_handler_changes_and_removes_untouched_overlay_fields() {
    let mut config = Config::default();
    config.config_path = PathBuf::from("/test/config.toml");
    config.chat_provider = Some("openai:before".into());
    let baseline = InferenceFields::from_config(&config);

    let overrides = CliInferenceOverrides {
        provider: Some("ollama".into()),
        model: Some("qwen3:8b".into()),
    };
    apply_overrides(&mut config, &overrides);
    let applied = InferenceFields::from_config(&config);
    config.coding_provider = Some("anthropic:explicit-change".into());

    baseline.restore_matching(&mut config, &applied);

    assert_eq!(config.default_model, baseline.default_model);
    assert_eq!(config.chat_provider, baseline.chat_provider);
    assert_eq!(config.reasoning_provider, baseline.reasoning_provider);
    assert_eq!(config.agentic_provider, baseline.agentic_provider);
    assert_eq!(
        config.coding_provider.as_deref(),
        Some("anthropic:explicit-change")
    );
}

#[test]
fn managed_provider_keeps_its_sentinel_route_and_uses_default_model() {
    let mut config = Config::default();

    apply_overrides(
        &mut config,
        &CliInferenceOverrides {
            provider: Some("openhuman".into()),
            model: Some("reasoning-v1".into()),
        },
    );

    assert_eq!(config.chat_provider.as_deref(), Some("openhuman"));
    assert_eq!(config.reasoning_provider, config.chat_provider);
    assert_eq!(config.default_model.as_deref(), Some("reasoning-v1"));
}

#[test]
fn lm_studio_alias_is_normalized_to_factory_route() {
    let mut config = Config::default();

    apply_overrides(
        &mut config,
        &CliInferenceOverrides {
            provider: Some("lm_studio".into()),
            model: Some("local-model".into()),
        },
    );

    assert_eq!(
        config.chat_provider.as_deref(),
        Some("lmstudio:local-model")
    );
}
