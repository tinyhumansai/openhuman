use super::*;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

fn test_config() -> Config {
    Config::default()
}

fn openai_entry(id: &str, slug: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: "OpenAI".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("gpt-4o".to_string()),
    }
}

#[test]
fn build_remote_provider_managed_uses_backend_id_and_default_model() {
    // Default config → chat workload resolves to the managed backend →
    // the managed default model.
    let config = test_config();
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .expect("remote provider should build");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
    assert_eq!(resolved.model, crate::config::MODEL_MANAGED_DEFAULT);
    assert!(!resolved.used_local, "used_local is always false");
}

#[test]
fn build_remote_provider_managed_follows_the_pinned_default_model() {
    // Triage's managed arm runs on the managed default model: a retired tier
    // slug in `default_model` is not a pin, a catalog id is.
    let mut config = test_config();
    config.default_model = Some("reasoning-v1".to_string());
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .expect("remote provider should build");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
    assert_eq!(resolved.model, crate::config::MODEL_MANAGED_DEFAULT);

    config.default_model = Some("openrouter/deepseek/deepseek-v4-pro".to_string());
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .expect("remote provider should build");
    assert_eq!(resolved.model, "openrouter/deepseek/deepseek-v4-pro");
    assert!(!resolved.used_local);
}

#[test]
fn build_remote_provider_falls_back_to_managed_when_chat_local() {
    // #1257: triage must never run on a local provider. A local
    // `chat_provider` falls back to the managed backend so a trigger
    // never errors because Ollama is down.
    let mut config = test_config();
    config.chat_provider = Some("ollama:llama3.2:3b".to_string());
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .expect("remote provider should build");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
    assert_eq!(resolved.model, crate::config::MODEL_MANAGED_DEFAULT);
    assert!(!resolved.used_local, "triage never goes local");
}

#[test]
fn build_remote_provider_forces_managed_for_local_cli_routes() {
    // #1257 (Codex P2): local CLI delegates (claude_agent_sdk / claude-code:)
    // are not caught by `is_local_provider_string`, but triage must not depend on
    // a local CLI — force the managed backend for them too.
    for route in [
        "claude_agent_sdk",
        "claude_agent_sdk:sonnet",
        "claude-code:opus",
    ] {
        let mut config = test_config();
        config.chat_provider = Some(route.to_string());
        let resolved =
            build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
                .unwrap_or_else(|e| panic!("route {route} should build, got {e}"));
        assert_eq!(
            resolved.provider_name, INFERENCE_BACKEND_ID,
            "route {route} must force managed backend"
        );
        assert_eq!(
            resolved.model,
            crate::config::MODEL_MANAGED_DEFAULT,
            "route {route} must run the managed default"
        );
        assert!(!resolved.used_local);
    }
}

#[test]
fn is_local_cli_route_classifies_cli_delegates_only() {
    assert!(is_local_cli_route("claude_agent_sdk"));
    assert!(is_local_cli_route("claude_agent_sdk:sonnet"));
    assert!(is_local_cli_route("claude-code:opus"));
    assert!(!is_local_cli_route("openai:gpt-4o"));
    assert!(!is_local_cli_route("openhuman"));
    assert!(!is_local_cli_route("ollama:phi3"));
}

#[test]
fn build_remote_provider_routes_through_byok_chat() {
    // A concrete BYOK cloud chat provider governs triage classification.
    let mut config = test_config();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o-mini".to_string());
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .expect("remote provider should build");
    assert_eq!(resolved.provider_name, "openai");
    assert_eq!(resolved.model, "gpt-4o-mini");
    assert!(!resolved.used_local);
}

#[test]
fn build_remote_provider_falls_back_when_byok_build_fails() {
    // A non-local, non-managed chat provider whose slug has no matching
    // `cloud_providers` entry fails to build — triage falls back to the managed
    // backend rather than erroring the turn.
    let mut config = test_config();
    config.chat_provider = Some("groq:llama3".to_string());
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .expect("remote provider should build");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
    assert_eq!(resolved.model, crate::config::MODEL_MANAGED_DEFAULT);
    assert!(!resolved.used_local);
}

#[tokio::test]
async fn resolve_provider_with_config_always_returns_remote() {
    // Even when runtime_enabled is true, triage must always use remote.
    let mut config = test_config();
    config.local_ai.runtime_enabled = true;
    let resolved =
        resolve_provider_with_config(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .expect("resolve should succeed");
    assert!(!resolved.used_local, "triage must never use local AI");
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
}

#[tokio::test]
async fn resolve_provider_with_config_returns_remote_when_local_disabled() {
    let mut config = test_config();
    config.local_ai.runtime_enabled = false;
    let resolved =
        resolve_provider_with_config(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .expect("resolve should succeed");
    assert!(!resolved.used_local);
    assert_eq!(resolved.provider_name, INFERENCE_BACKEND_ID);
}
