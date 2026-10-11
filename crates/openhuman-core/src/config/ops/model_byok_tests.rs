//! `complete_byok_route` — making `inference_url` + `api_key` actually route.
//!
//! The regression these cover is a write the API accepted and a *different*
//! subsystem rejected one call later: setting the two documented BYOK fields
//! saved fine and then every turn died with `BYOK_INCOMPLETE`, because role
//! resolution goes through `cloud_providers` and nothing had put the endpoint
//! there.

use super::*;

#[test]
fn provider_ca_updates_validate_bundles_and_normalize_slugs() {
    let error = collect_provider_ca_certs([(" team ", Some("not a certificate"))])
        .expect_err("invalid CA material must fail before applying settings");
    assert!(error.contains("CA bundle"));

    let certs = collect_provider_ca_certs([(" team ", Some(""))]).unwrap();
    assert_eq!(certs.get("team").map(String::as_str), Some(""));
}
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::Config;

fn config_with(inference_url: Option<&str>, api_key: Option<&str>, model: Option<&str>) -> Config {
    let mut config = Config::default();
    config.inference_url = inference_url.map(str::to_string);
    config.api_key = api_key.map(str::to_string);
    config.default_model = model.map(str::to_string);
    config
}

fn entry(slug: &str, endpoint: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: slug.to_string(),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint: endpoint.to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: None,
    }
}

#[test]
fn registers_a_provider_and_pins_the_agent_turn_roles() {
    let mut config = config_with(
        Some("https://openrouter.ai/api/v1"),
        Some("sk-test"),
        Some("deepseek/deepseek-v4.1-flash"),
    );

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    let registered = config
        .cloud_providers
        .iter()
        .find(|e| e.slug == BYOK_INFERENCE_SLUG)
        .expect("a provider entry for the configured inference_url");
    assert_eq!(registered.endpoint, "https://openrouter.ai/api/v1");

    let expected = format!("{BYOK_INFERENCE_SLUG}:deepseek/deepseek-v4.1-flash");
    assert_eq!(config.chat_provider.as_deref(), Some(expected.as_str()));
    assert_eq!(
        config.reasoning_provider.as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(config.agentic_provider.as_deref(), Some(expected.as_str()));
    assert_eq!(config.coding_provider.as_deref(), Some(expected.as_str()));
}

#[test]
fn an_endpoint_without_a_key_is_left_alone() {
    // An endpoint with no credential is a partial statement; completing it
    // would send the turn somewhere the caller never authenticated.
    let mut config = config_with(Some("https://openrouter.ai/api/v1"), None, Some("m"));

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert!(config.cloud_providers.is_empty());
    assert!(config.chat_provider.is_none());
}

#[test]
fn a_key_without_an_endpoint_is_left_alone() {
    let mut config = config_with(None, Some("sk-test"), Some("m"));

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert!(config.cloud_providers.is_empty());
    assert!(config.chat_provider.is_none());
}

#[test]
fn a_blank_default_model_registers_nothing() {
    // The provider grammar is `<slug>:<model>`; pinning a role to `<slug>:`
    // would trade a working default for a resolution failure.
    let mut config = config_with(Some("https://openrouter.ai/api/v1"), Some("sk-test"), None);

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert!(config.cloud_providers.is_empty());
    assert!(config.chat_provider.is_none());
}

#[test]
fn an_existing_entry_for_the_same_endpoint_is_reused_not_duplicated() {
    let mut config = config_with(
        Some("https://openrouter.ai/api/v1/"),
        Some("sk-test"),
        Some("m"),
    );
    config
        .cloud_providers
        .push(entry("my-openrouter", "https://openrouter.ai/api/v1"));

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert_eq!(config.cloud_providers.len(), 1, "no duplicate entry");
    assert_eq!(config.cloud_providers[0].slug, "my-openrouter");
    // Roles resolve through the caller's own slug, not a synthesised one.
    assert_eq!(config.chat_provider.as_deref(), Some("my-openrouter:m"));
}

#[test]
fn a_role_pinned_by_the_same_patch_is_not_overwritten() {
    let mut config = config_with(
        Some("https://openrouter.ai/api/v1"),
        Some("sk-test"),
        Some("m"),
    );
    config.chat_provider = Some("ollama:qwen2.5".to_string());
    let explicit = ExplicitRolePins {
        chat: true,
        ..Default::default()
    };

    complete_byok_route(&mut config, &explicit);

    assert_eq!(config.chat_provider.as_deref(), Some("ollama:qwen2.5"));
    // The roles the patch said nothing about still get completed.
    assert_eq!(
        config.agentic_provider.as_deref(),
        Some(format!("{BYOK_INFERENCE_SLUG}:m").as_str())
    );
}

#[test]
fn a_role_already_pointing_somewhere_deliberate_is_not_repointed() {
    // Not named by this patch, but not "unspoken for" either — a previously
    // configured local runtime must not be silently moved onto the BYOK
    // endpoint.
    let mut config = config_with(
        Some("https://openrouter.ai/api/v1"),
        Some("sk-test"),
        Some("m"),
    );
    config.coding_provider = Some("ollama:qwen2.5-coder".to_string());

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert_eq!(
        config.coding_provider.as_deref(),
        Some("ollama:qwen2.5-coder")
    );
    assert_eq!(
        config.chat_provider.as_deref(),
        Some(format!("{BYOK_INFERENCE_SLUG}:m").as_str())
    );
}

#[test]
fn the_managed_cloud_sentinel_counts_as_unspoken_for() {
    let mut config = config_with(
        Some("https://openrouter.ai/api/v1"),
        Some("sk-test"),
        Some("m"),
    );
    config.chat_provider = Some("cloud".to_string());

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert_eq!(
        config.chat_provider.as_deref(),
        Some(format!("{BYOK_INFERENCE_SLUG}:m").as_str())
    );
}

#[test]
fn clearing_inference_url_stops_completing_anything() {
    let mut config = config_with(Some("   "), Some("sk-test"), Some("m"));

    complete_byok_route(&mut config, &ExplicitRolePins::default());

    assert!(config.cloud_providers.is_empty());
    assert!(config.chat_provider.is_none());
}
