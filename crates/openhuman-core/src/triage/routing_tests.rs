use super::*;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

async fn test_config() -> Config {
    let config = Config::default();
    crate::credentials::api_key::store_api_key(&config, "test-session-fixture-key")
        .await
        .expect("store triage test credential");
    config
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

#[tokio::test]
async fn managed_route_without_a_registered_provider_fails_closed() {
    let config = test_config().await;
    let error = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .err()
        .expect("bare core has no hosted model source");
    assert!(error.to_string().contains("BACKEND_UNAVAILABLE"));
}

#[tokio::test]
async fn managed_route_does_not_invent_a_model_when_provider_policy_is_absent() {
    let mut config = test_config().await;
    config.default_model = Some("reasoning-v1".to_string());
    let error = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .err()
        .expect("retired pin is not a concrete model source");
    assert!(error.to_string().contains("BACKEND_UNAVAILABLE"));

    config.default_model = Some("openrouter/deepseek/deepseek-v4-pro".to_string());
    assert!(
        build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .is_err(),
        "an explicit pin still needs a registered hosted constructor"
    );
}

#[tokio::test]
async fn build_remote_provider_falls_back_to_managed_when_chat_local() {
    // #1257: triage must never run on a local provider. A local
    // `chat_provider` falls back to the managed backend so a trigger
    // never errors because Ollama is down.
    let mut config = test_config().await;
    config.chat_provider = Some("ollama:llama3.2:3b".to_string());
    assert!(
        build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .is_err(),
        "triage's managed fallback requires an installed provider"
    );
}

#[tokio::test]
async fn build_remote_provider_forces_managed_for_local_cli_routes() {
    // #1257 (Codex P2): local CLI delegates (claude_agent_sdk / claude-code:)
    // are not caught by `is_local_provider_string`, but triage must not depend on
    // a local CLI — force the managed backend for them too.
    for route in [
        "claude_agent_sdk",
        "claude_agent_sdk:sonnet",
        "claude-code:opus",
    ] {
        let mut config = test_config().await;
        config.chat_provider = Some(route.to_string());
        assert!(
            build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
                .await
                .is_err(),
            "route {route} must fail closed without hosted construction policy"
        );
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

#[tokio::test]
async fn build_remote_provider_routes_through_byok_chat() {
    // A concrete BYOK cloud chat provider governs triage classification.
    let mut config = test_config().await;
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o-mini".to_string());
    let resolved = build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
        .await
        .expect("remote provider should build");
    assert_eq!(resolved.provider_name, "openai");
    assert_eq!(resolved.model, "gpt-4o-mini");
    assert!(!resolved.used_local);
}

#[tokio::test]
async fn build_remote_provider_reports_missing_managed_fallback_policy() {
    // A non-local, non-managed chat provider whose slug has no matching
    // `cloud_providers` entry fails to build — triage falls back to the managed
    // backend rather than erroring the turn.
    let mut config = test_config().await;
    config.chat_provider = Some("groq:llama3".to_string());
    assert!(
        build_remote_provider(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .is_err(),
        "fallback cannot fabricate a managed model"
    );
}

#[tokio::test]
async fn resolve_provider_with_config_requires_a_registered_remote_model_source() {
    // Even when runtime_enabled is true, triage must always use remote.
    let mut config = test_config().await;
    config.local_ai.runtime_enabled = true;
    assert!(
        resolve_provider_with_config(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn resolve_provider_with_config_without_provider_does_not_invent_a_route() {
    let mut config = test_config().await;
    config.local_ai.runtime_enabled = false;
    assert!(
        resolve_provider_with_config(&crate::core::runtime::DispatchCtx::ambient(), &config)
            .await
            .is_err()
    );
}
