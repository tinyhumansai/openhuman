use super::*;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::credentials::AuthService;
use crate::inference::provider::factory::{BYOK_INCOMPLETE_SENTINEL, CLAUDE_AGENT_SDK_PROVIDER};
use tempfile::TempDir;

fn entry(slug: &str, endpoint: &str, auth_style: AuthStyle) -> CloudProviderCreds {
    CloudProviderCreds {
        id: format!("p_{slug}"),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style,
        ..Default::default()
    }
}

fn config(tmp: &TempDir, providers: Vec<CloudProviderCreds>) -> Config {
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config.cloud_providers = providers;
    config
}

async fn store_key(config: &Config, slug: &str, token: &str) {
    AuthService::from_config(config)
        .store_provider_token(
            &format!("provider:{slug}"),
            "default",
            token,
            Default::default(),
            true,
        )
        .await
        .expect("store provider token");
}

fn header<'a>(request: &'a DiscoveryRequest, name: &str) -> Option<&'a str> {
    request
        .headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[tokio::test]
async fn openrouter_slug_targets_its_models_listing_with_bearer() {
    let tmp = TempDir::new().unwrap();
    let config = config(
        &tmp,
        vec![entry(
            "openrouter",
            "https://openrouter.ai/api/v1",
            AuthStyle::Bearer,
        )],
    );
    store_key(&config, "openrouter", "sk-or-test").await;

    let request = model_limits_request(
        "chat",
        "openrouter:deepseek/deepseek-v4.1-flash",
        "deepseek/deepseek-v4.1-flash",
        &config,
    )
    .await
    .expect("cloud slug is discoverable");
    assert_eq!(request.endpoint, "https://openrouter.ai/api/v1");
    assert_eq!(request.model, "deepseek/deepseek-v4.1-flash");
    assert_eq!(
        request.effective_listing_url(),
        "https://openrouter.ai/api/v1/models"
    );
    assert_eq!(header(&request, "Authorization"), Some("Bearer sk-or-test"));
    // OpenHuman's OpenRouter routing sorts by price with fallbacks on, which
    // pins no provider, so the model-level window applies.
    assert!(request.pinned_providers.is_empty());
}

#[tokio::test]
async fn temperature_suffix_is_not_part_of_the_model() {
    let tmp = TempDir::new().unwrap();
    let config = config(
        &tmp,
        vec![entry(
            "proxy",
            "http://127.0.0.1:9911/v1",
            AuthStyle::Bearer,
        )],
    );
    let request = model_limits_request("chat", "proxy:deepseek/x@0.2", "deepseek/x", &config)
        .await
        .expect("discoverable");
    assert_eq!(request.endpoint, "http://127.0.0.1:9911/v1");
    assert_eq!(request.model, "deepseek/x");
}

#[tokio::test]
async fn anthropic_slug_uses_x_api_key() {
    let tmp = TempDir::new().unwrap();
    let config = config(
        &tmp,
        vec![entry(
            "anthropic",
            "https://api.anthropic.com/v1",
            AuthStyle::Anthropic,
        )],
    );
    store_key(&config, "anthropic", "sk-ant-test").await;
    let request = model_limits_request(
        "chat",
        "anthropic:claude-sonnet-4-6",
        "claude-sonnet-4-6",
        &config,
    )
    .await
    .expect("discoverable");
    assert_eq!(header(&request, "x-api-key"), Some("sk-ant-test"));
    assert_eq!(header(&request, "Authorization"), None);
}

#[tokio::test]
async fn local_and_subprocess_routes_are_not_discovered() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp, Vec::new());
    for provider in [
        "ollama:llama3.1:8b",
        BYOK_INCOMPLETE_SENTINEL,
        CLAUDE_AGENT_SDK_PROVIDER,
    ] {
        assert!(
            model_limits_request("chat", provider, "m", &config)
                .await
                .is_none(),
            "{provider}"
        );
    }
}

#[tokio::test]
async fn unknown_slug_is_not_discovered() {
    let tmp = TempDir::new().unwrap();
    let config = config(&tmp, Vec::new());
    assert!(model_limits_request("chat", "nope:model", "model", &config)
        .await
        .is_none());
}
