use super::*;
use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::inference::provider::traits::{ChatMessage, ChatRequest, ProviderDelta};
use tempfile::TempDir;

fn config_with_providers(providers: Vec<CloudProviderCreds>) -> Config {
    let mut c = Config::default();
    c.cloud_providers = providers;
    c
}

fn config_with_providers_in_tempdir(tmp: &TempDir, providers: Vec<CloudProviderCreds>) -> Config {
    let mut c = config_with_providers(providers);
    c.workspace_dir = tmp.path().join("workspace");
    c.config_path = tmp.path().join("config.toml");
    c
}

fn oh_entry(id: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "openhuman".to_string(),
        label: "OpenHuman".to_string(),
        endpoint: "https://api.openhuman.ai/v1".to_string(),
        auth_style: AuthStyle::OpenhumanJwt,
        ..Default::default()
    }
}

fn openai_entry(id: &str, slug: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: "OpenAI".to_string(),
        endpoint: "https://api.openai.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("gpt-4o".to_string()),
        ..Default::default()
    }
}

fn anthropic_entry(id: &str, slug: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: slug.to_string(),
        label: "Anthropic".to_string(),
        endpoint: "https://api.anthropic.com/v1".to_string(),
        auth_style: AuthStyle::Anthropic,
        default_model: Some("claude-sonnet-4-6".to_string()),
        ..Default::default()
    }
}

#[test]
fn openhuman_literal() {
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("reasoning", "openhuman", &config)
        .expect("openhuman literal must build");
    assert!(!model.is_empty(), "model must not be empty");
}

#[test]
fn cloud_no_providers_falls_back_to_openhuman() {
    let config = Config::default();
    let result = create_chat_provider_from_string("reasoning", "cloud", &config);
    assert!(
        result.is_ok(),
        "cloud fallback must succeed: {:?}",
        result.err()
    );
}

#[test]
fn direct_cloud_sentinel_resolves_to_primary_custom_provider() {
    let mut config = config_with_providers(vec![oh_entry("p_oh"), openai_entry("p_oai", "openai")]);
    config.primary_cloud = Some("p_oai".to_string());

    let (_, model) =
        create_chat_provider_from_string("reasoning", "cloud", &config).expect("build");
    assert_eq!(model, "gpt-4o");
}

#[test]
fn openhuman_slug_routes_to_backend() {
    let config = config_with_providers(vec![oh_entry("p_oh")]);
    let (_, model) =
        create_chat_provider_from_string("reasoning", "openhuman:", &config).expect("build");
    assert!(!model.is_empty());
}

#[test]
fn openai_slug_model() {
    let config = config_with_providers(vec![openai_entry("p_oai", "openai")]);
    let (_, model) = create_chat_provider_from_string("agentic", "openai:gpt-4o-mini", &config)
        .expect("openai:<model> must build");
    assert_eq!(model, "gpt-4o-mini");
}

#[test]
fn anthropic_slug_model() {
    let config = config_with_providers(vec![anthropic_entry("p_ant", "anthropic")]);
    let (_, model) =
        create_chat_provider_from_string("coding", "anthropic:claude-sonnet-4-6", &config)
            .expect("anthropic:<model> must build");
    assert_eq!(model, "claude-sonnet-4-6");
}

#[test]
fn openrouter_slug_model() {
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_or".to_string(),
        slug: "openrouter".to_string(),
        label: "OpenRouter".to_string(),
        endpoint: "https://openrouter.ai/api/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("openai/gpt-4o".to_string()),
        ..Default::default()
    });
    let (_, model) =
        create_chat_provider_from_string("agentic", "openrouter:meta-llama/llama-3.1-8b", &config)
            .expect("openrouter:<model> must build");
    assert_eq!(model, "meta-llama/llama-3.1-8b");
}

#[test]
fn custom_provider_remaps_abstract_tier_to_concrete_default_model() {
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_ds".to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-v4-pro".to_string()),
        ..Default::default()
    });

    let (_, model) =
        create_chat_provider_from_string("reasoning", "deepseek:reasoning-v1", &config)
            .expect("abstract tier should remap to concrete default model");
    assert_eq!(model, "deepseek-v4-pro");
}

#[test]
fn custom_provider_rejects_abstract_tier_without_concrete_default_model() {
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_ds".to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: None,
        ..Default::default()
    });

    // Can't use `.expect_err(..)` here because `Box<dyn Provider>` doesn't
    // implement `Debug`, so the success arm has no Debug to print.
    let err = match create_chat_provider_from_string("reasoning", "deepseek:reasoning-v1", &config)
    {
        Ok(_) => panic!("abstract tier without concrete provider default should fail"),
        Err(e) => e,
    };
    assert!(err.to_string().contains("abstract tier"));
}

#[test]
fn orcarouter_slug_model() {
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_oc".to_string(),
        slug: "orcarouter".to_string(),
        label: "OrcaRouter".to_string(),
        endpoint: "https://api.orcarouter.ai/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("orcarouter/auto".to_string()),
        ..Default::default()
    });
    let (_, model) =
        create_chat_provider_from_string("agentic", "orcarouter:orcarouter/auto", &config)
            .expect("orcarouter:<model> must build");
    assert_eq!(model, "orcarouter/auto");
}

#[test]
fn orcarouter_legacy_type_seeds_defaults() {
    use crate::openhuman::config::schema::cloud_providers::migrate_legacy_fields;
    let mut entry = CloudProviderCreds {
        id: "p_oc_legacy".to_string(),
        legacy_type: Some("orcarouter".to_string()),
        ..Default::default()
    };
    migrate_legacy_fields(&mut entry);
    assert_eq!(entry.slug, "orcarouter");
    assert_eq!(entry.label, "OrcaRouter");
    assert_eq!(entry.endpoint, "https://api.orcarouter.ai/v1");
    assert_eq!(entry.auth_style, AuthStyle::Bearer);
}

#[test]
fn ollama_prefix() {
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("heartbeat", "ollama:llama3.1:8b", &config)
        .expect("ollama:<model> must build");
    assert_eq!(model, "llama3.1:8b");
}

#[test]
fn lmstudio_prefix() {
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://127.0.0.1:1234".to_string());
    let (_, model) =
        create_chat_provider_from_string("heartbeat", "lmstudio:google/gemma-4-e4b", &config)
            .expect("lmstudio:<model> must build");
    assert_eq!(model, "google/gemma-4-e4b");
}

#[test]
fn temperature_suffix_is_stripped_from_model_id() {
    // The `@<temp>` suffix is informational for the factory — the model id sent
    // upstream must not include it, or providers will 404 on an unknown model.
    let config = Config::default();
    let (_, model) =
        create_chat_provider_from_string("heartbeat", "ollama:llama3.1:8b@0.2", &config)
            .expect("ollama:<model>@<temp> must build");
    assert_eq!(
        model, "llama3.1:8b",
        "temperature suffix must not leak into the dispatched model id"
    );
}

#[test]
fn malformed_temperature_suffix_kept_as_part_of_model_id() {
    // If the tail after `@` isn't a number, treat the whole string as the model
    // id rather than silently dropping a chunk of it.
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("heartbeat", "ollama:llama3@beta", &config)
        .expect("ollama:<model>@<garbage> must still build");
    assert_eq!(model, "llama3@beta");
}

#[tokio::test]
async fn ollama_provider_does_not_require_api_key() {
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://127.0.0.1:9".to_string());
    let (provider, model) =
        create_chat_provider_from_string("heartbeat", "ollama:llama3.1:8b", &config)
            .expect("ollama:<model> must build");

    let err = provider
        .chat_with_system(None, "hello", &model, 0.0)
        .await
        .expect_err("unreachable local Ollama should still attempt a transport call");
    let msg = err.to_string();
    assert!(
        !msg.contains("API key not set"),
        "ollama path must not fail on missing key: {msg}"
    );
}

#[tokio::test]
async fn lmstudio_provider_without_api_key_does_not_require_credentials() {
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://127.0.0.1:9/v1".to_string());
    let (provider, model) =
        create_chat_provider_from_string("heartbeat", "lmstudio:test-model", &config)
            .expect("lmstudio:<model> must build");

    let err = provider
        .chat_with_system(None, "hello", &model, 0.0)
        .await
        .expect_err("unreachable local LM Studio should still attempt a transport call");
    let msg = err.to_string();
    assert!(
        !msg.contains("API key not set"),
        "lmstudio path must not fail on missing key: {msg}"
    );
}

#[test]
fn all_workloads_default_to_openhuman() {
    let config = Config::default();
    for role in &[
        "chat",
        "reasoning",
        "agentic",
        "coding",
        "memory",
        "embeddings",
        "heartbeat",
        "learning",
        "subconscious",
    ] {
        assert_eq!(
            provider_for_role(role, &config),
            "openhuman",
            "role={role} must default to openhuman"
        );
    }
}

// Regression: the `chat` workload was added to the UI + config schema (#2152)
// but `provider_for_role` was not extended, so every chat message silently
// routed to the OpenHuman backend regardless of the user's `chat_provider`
// configuration. Keep this test alongside the other override checks so the
// arm can't drop out again.
#[test]
fn chat_workload_override_respected() {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4".to_string());
    assert_eq!(provider_for_role("chat", &config), "openai:gpt-4");
}

#[test]
fn workload_override_respected() {
    let mut config = Config::default();
    config.heartbeat_provider = Some("ollama:llama3.2:3b".to_string());
    assert_eq!(
        provider_for_role("heartbeat", &config),
        "ollama:llama3.2:3b"
    );
    assert_eq!(provider_for_role("reasoning", &config), "openhuman");
}

#[test]
fn create_chat_provider_uses_role() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.reasoning_provider = Some("openai:gpt-4o-mini".to_string());
    let (_, model) =
        create_chat_provider("reasoning", &config).expect("create_chat_provider must succeed");
    assert_eq!(model, "gpt-4o-mini");
}

#[test]
fn unknown_slug_rejected() {
    let config = Config::default();
    let err = create_chat_provider_from_string("reasoning", "groq:llama3", &config)
        .err()
        .expect("unknown slug must fail");
    assert!(
        err.to_string()
            .contains("no cloud provider configured for slug"),
        "{err}"
    );
}

#[test]
fn bare_string_without_colon_rejected() {
    let config = Config::default();
    let err = create_chat_provider_from_string("reasoning", "openai", &config)
        .err()
        .expect("bare string must fail");
    assert!(
        err.to_string().contains("unrecognised provider string"),
        "{err}"
    );
}

#[test]
fn empty_model_in_ollama_rejected() {
    let config = Config::default();
    let err = create_chat_provider_from_string("reasoning", "ollama:", &config)
        .err()
        .expect("empty model must fail");
    assert!(err.to_string().contains("empty model"), "{err}");
}

#[test]
fn missing_slug_for_openai_gives_clear_error() {
    let config = Config::default();
    let err = create_chat_provider_from_string("reasoning", "openai:gpt-4o", &config)
        .err()
        .expect("missing slug must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("no cloud provider configured for slug 'openai'"),
        "{msg}"
    );
}

#[tokio::test]
async fn cloud_provider_without_stored_key_fails_with_actionable_error() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    let (provider, model) = create_chat_provider_from_string("reasoning", "openai:gpt-4o", &config)
        .expect("provider should build without eagerly requiring credentials");

    let err = provider
        .chat_with_system(None, "hello", &model, 0.0)
        .await
        .expect_err("missing key should fail at call time");
    assert!(
        err.to_string().contains("API key not set"),
        "expected missing-key guidance, got: {err}"
    );
}

#[tokio::test]
async fn cloud_provider_with_auth_none_does_not_require_api_key() {
    let tmp = TempDir::new().expect("tempdir");
    let mut entry = openai_entry("p_proxy", "proxy");
    entry.auth_style = AuthStyle::None;
    entry.endpoint = "http://127.0.0.1:9".to_string();
    let config = config_with_providers_in_tempdir(&tmp, vec![entry]);
    let (provider, model) = create_chat_provider_from_string("reasoning", "proxy:gpt-oss", &config)
        .expect("auth:none provider must build");

    let err = provider
        .chat_with_system(None, "hello", &model, 0.0)
        .await
        .expect_err("unreachable auth:none endpoint should attempt transport");
    let msg = err.to_string();
    assert!(
        !msg.contains("API key not set"),
        "auth:none provider must not fail on missing key: {msg}"
    );
}

#[tokio::test]
async fn cloud_provider_with_malformed_endpoint_surfaces_url_error() {
    let tmp = TempDir::new().expect("tempdir");
    let mut entry = openai_entry("p_bad", "openai");
    entry.endpoint = "://not a url".to_string();
    let config = config_with_providers_in_tempdir(&tmp, vec![entry]);
    let auth = AuthService::from_config(&config);
    auth.store_provider_token(
        "provider:openai",
        "default",
        "sk-test",
        Default::default(),
        true,
    )
    .expect("store provider token");

    let (provider, model) = create_chat_provider_from_string("reasoning", "openai:gpt-4o", &config)
        .expect("provider should still build");

    let err = provider
        .chat_with_system(None, "hello", &model, 0.0)
        .await
        .expect_err("malformed endpoint should fail at request build/send time");
    let msg = err.to_string().to_ascii_lowercase();
    assert!(
        msg.contains("builder error")
            || msg.contains("relative url without a base")
            || msg.contains("empty host")
            || msg.contains("invalid port"),
        "expected malformed-url style error, got: {msg}"
    );
}

#[test]
fn primary_cloud_defaults_to_openhuman_when_no_providers() {
    let config = Config::default();
    assert!(create_chat_provider("reasoning", &config).is_ok());
}

#[test]
fn cloud_sentinel_resolves_to_primary_custom_provider() {
    let mut config = config_with_providers(vec![oh_entry("p_oh"), openai_entry("p_oai", "openai")]);
    config.primary_cloud = Some("p_oai".to_string());

    assert_eq!(provider_for_role("reasoning", &config), "openai:gpt-4o");

    let (_, model) =
        create_chat_provider("reasoning", &config).expect("primary custom provider must build");
    assert_eq!(model, "gpt-4o");
}

#[test]
fn legacy_inference_url_custom_provider_wins_over_openhuman_primary_for_unset_role() {
    let mut custom = openai_entry("p_custom", "custom");
    custom.endpoint = "https://api.example.com/v1/".to_string();
    custom.default_model = Some("gpt-4o-mini".to_string());

    let mut config = config_with_providers(vec![oh_entry("p_oh"), custom]);
    config.primary_cloud = Some("p_oh".to_string());
    config.inference_url = Some("https://api.example.com/v1".to_string());

    assert_eq!(
        provider_for_role("reasoning", &config),
        "custom:gpt-4o-mini"
    );
}

#[test]
fn legacy_inference_url_without_matching_provider_returns_byok_sentinel() {
    // BYOK intent: primary is OpenHuman but inference_url points at a custom
    // endpoint with no matching cloud_providers entry. Must fail closed — do
    // NOT silently route through the managed backend.
    let mut other = openai_entry("p_other", "other");
    other.endpoint = "https://other.example.com/v1".to_string();

    let mut config = config_with_providers(vec![oh_entry("p_oh"), other]);
    config.primary_cloud = Some("p_oh".to_string());
    config.inference_url = Some("https://api.example.com/v1".to_string());

    assert_eq!(
        provider_for_role("reasoning", &config),
        BYOK_INCOMPLETE_SENTINEL
    );
}

#[test]
fn hosted_endpoint_entry_is_treated_as_openhuman_backend() {
    let mut hosted = openai_entry("p_hosted", "custom-hosted");
    hosted.endpoint = "https://staging-api.tinyhumans.ai/openai/v1".to_string();
    hosted.auth_style = AuthStyle::Bearer;

    let mut config = config_with_providers(vec![hosted]);
    config.primary_cloud = Some("p_hosted".to_string());

    assert_eq!(provider_for_role("reasoning", &config), "openhuman");
}

#[test]
fn explicit_openhuman_route_ignores_legacy_inference_url() {
    let mut custom = openai_entry("p_custom", "custom");
    custom.endpoint = "https://api.example.com/v1".to_string();

    let mut config = config_with_providers(vec![oh_entry("p_oh"), custom]);
    config.primary_cloud = Some("p_oh".to_string());
    config.inference_url = Some("https://api.example.com/v1".to_string());
    config.reasoning_provider = Some("openhuman".to_string());

    assert_eq!(provider_for_role("reasoning", &config), "openhuman");
}

#[test]
fn summarization_aliases_memory_provider() {
    let mut config = Config::default();
    config.memory_provider = Some("ollama:llama3.1:8b".to_string());
    assert_eq!(provider_for_role("memory", &config), "ollama:llama3.1:8b");
    assert_eq!(
        provider_for_role("summarization", &config),
        "ollama:llama3.1:8b",
        "summarization must alias memory_provider"
    );
}

#[test]
fn summarization_defaults_to_openhuman_like_memory() {
    let config = Config::default();
    assert_eq!(provider_for_role("memory", &config), "openhuman");
    assert_eq!(provider_for_role("summarization", &config), "openhuman");
}

#[test]
fn unknown_workload_falls_back_to_openhuman() {
    let config = Config::default();
    assert_eq!(
        provider_for_role("nope-not-a-workload", &config),
        "openhuman"
    );
    assert_eq!(provider_for_role("", &config), "openhuman");
}

#[test]
fn openhuman_backend_uses_config_path_parent_as_state_dir() {
    let mut config = Config::default();
    config.config_path = std::path::PathBuf::from("/tmp/oh-test-workspace/config.toml");
    let (_provider, model) = create_chat_provider("reasoning", &config)
        .expect("openhuman backend must build with no cloud_providers");
    assert!(!model.is_empty(), "model must be set")
}

// ── verify_session_active tests ──────────────────────────────────────

/// Helper: build a Config whose `config_path` lives inside a tempdir.
fn config_in_tempdir(tmp: &TempDir) -> Config {
    let mut c = Config::default();
    c.config_path = tmp.path().join("config.toml");
    c
}

async fn discover_live_lmstudio_model() -> anyhow::Result<String> {
    if let Ok(model) = std::env::var("OPENHUMAN_LIVE_LMSTUDIO_MODEL") {
        let trimmed = model.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let body: serde_json::Value = reqwest::get("http://127.0.0.1:1234/v1/models")
        .await?
        .json()
        .await?;
    body["data"]
        .as_array()
        .and_then(|models| {
            models.iter().find_map(|item| {
                let id = item.get("id")?.as_str()?.trim();
                if id.is_empty() || id.contains("embed") {
                    None
                } else {
                    Some(id.to_string())
                }
            })
        })
        .ok_or_else(|| anyhow::anyhow!("no non-embedding LM Studio model discovered"))
}

async fn discover_live_ollama_model() -> anyhow::Result<String> {
    if let Ok(model) = std::env::var("OPENHUMAN_LIVE_OLLAMA_MODEL") {
        let trimmed = model.trim();
        if !trimmed.is_empty() {
            return Ok(trimmed.to_string());
        }
    }

    let body: serde_json::Value = reqwest::get("http://127.0.0.1:11434/api/tags")
        .await?
        .json()
        .await?;
    body["models"]
        .as_array()
        .and_then(|models| {
            models.iter().find_map(|item| {
                let name = item.get("name")?.as_str()?.trim();
                if name.is_empty() || name.contains("embed") {
                    None
                } else {
                    Some(name.to_string())
                }
            })
        })
        .ok_or_else(|| anyhow::anyhow!("no non-embedding Ollama model discovered"))
}

#[test]
fn verify_session_active_rejects_when_no_session_token() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in_tempdir(&tmp);
    let err = verify_session_active(&config).expect_err("should fail without session token");
    let msg = err.to_string();
    assert!(
        msg.contains("SESSION_EXPIRED"),
        "expected SESSION_EXPIRED, got: {msg}",
    );
}

#[test]
fn verify_session_active_rejects_when_token_is_empty() {
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_in_tempdir(&tmp);
    let auth = AuthService::new(tmp.path(), config.secrets.encrypt);
    auth.store_provider_token("app-session", "default", "", Default::default(), false)
        .expect("store empty token");
    let err = verify_session_active(&config).expect_err("should reject empty token");
    assert!(
        err.to_string().contains("SESSION_EXPIRED"),
        "expected SESSION_EXPIRED, got: {err}",
    );
}

#[test]
fn verify_session_active_passes_when_session_token_present() {
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_in_tempdir(&tmp);
    let auth = AuthService::new(tmp.path(), config.secrets.encrypt);
    auth.store_provider_token(
        "app-session",
        "default",
        "fake-jwt-token",
        Default::default(),
        false,
    )
    .expect("store session token");
    assert!(
        verify_session_active(&config).is_ok(),
        "should pass when session token exists",
    );
}

#[test]
fn verify_session_active_called_for_custom_provider_not_for_openhuman() {
    // openhuman backend must always build (no session gate applied).
    let config = Config::default();
    assert!(create_chat_provider_from_string("reasoning", "openhuman", &config).is_ok(),);
    // Verify that when a custom provider is tried without a session,
    // we'd get blocked (this test exercises the non-#[cfg(test)] path
    // by directly calling verify_session_active).
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in_tempdir(&tmp);
    let _ = create_chat_provider_from_string("reasoning", "ollama:llama3", &config);
    // Under #[cfg(test)] the gate is skipped, so this succeeds.
    // We assert the gate *would* fire by testing verify_session_active directly.
    assert!(
        verify_session_active(&config).is_err(),
        "verify_session_active must reject config without session",
    );
}

#[test]
fn lookup_key_for_slug_routes_openai_oauth_lookup_path() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_in_tempdir(&tmp);
    let auth = AuthService::new(tmp.path(), config.secrets.encrypt);
    auth.store_provider_token(
        "provider:openai",
        "default",
        "sk-openai",
        Default::default(),
        true,
    )
    .expect("store openai token");

    let token = lookup_key_for_slug("openai", &config).expect("lookup openai token");

    assert_eq!(token, "sk-openai");
}

// ── is_known_openhuman_tier ───────────────────────────────────────────────────

#[test]
fn known_tiers_pass() {
    for tier in [
        "reasoning-v1",
        "chat-v1",
        "agentic-v1",
        "coding-v1",
        "reasoning-quick-v1",
    ] {
        assert!(
            is_known_openhuman_tier(tier),
            "expected tier '{tier}' to be recognized"
        );
    }
}

#[test]
fn known_hints_pass() {
    assert!(is_known_openhuman_tier("hint:reasoning"));
    assert!(is_known_openhuman_tier("hint:chat"));
    assert!(is_known_openhuman_tier("hint:agentic"));
    assert!(is_known_openhuman_tier("hint:coding"));
}

#[test]
fn invalid_models_fail() {
    assert!(!is_known_openhuman_tier("deepseek-v4-pro"));
    assert!(!is_known_openhuman_tier("claude-opus-4-7"));
    assert!(!is_known_openhuman_tier("gpt-4o"));
    assert!(!is_known_openhuman_tier(""));
    assert!(!is_known_openhuman_tier("reasoning-v2"));
    // Unrecognized `hint:*` values must NOT be accepted — the factory only
    // translates the four hints above, so any other `hint:*` string would
    // otherwise be forwarded to the backend and rejected with HTTP 400.
    assert!(!is_known_openhuman_tier("hint:garbage"));
    assert!(!is_known_openhuman_tier("hint:reasoning-quick"));
    assert!(!is_known_openhuman_tier("hint:"));
}

#[test]
fn make_openhuman_backend_forwards_unknown_hint_verbatim() {
    // Unrecognised hint:* strings (e.g. hint:reaction for lightweight models)
    // must be forwarded to the backend unchanged. The backend is authoritative
    // over which hint values it accepts; the factory only translates the four
    // canonical hints (reasoning/chat/agentic/coding).
    for hint in ["hint:reaction", "hint:garbage", "hint:summarization"] {
        let mut config = Config::default();
        config.default_model = Some(hint.to_string());
        let (_, model) = make_openhuman_backend(&config).expect("factory should succeed");
        assert_eq!(model, hint, "hint '{hint}' should pass through unchanged");
    }
}

#[test]
fn make_openhuman_backend_falls_back_for_invalid_model() {
    // An invalid default_model must not be forwarded to the backend.
    // The factory must silently fall back to reasoning-v1 (the platform default).
    let mut config = Config::default();
    config.default_model = Some("deepseek-v4-pro".to_string());
    let (_, model) = make_openhuman_backend(&config).expect("factory should succeed");
    assert_eq!(
        model,
        crate::openhuman::config::MODEL_REASONING_V1,
        "invalid default_model should fall back to MODEL_REASONING_V1"
    );
}

#[test]
fn make_openhuman_backend_keeps_valid_tier() {
    let mut config = Config::default();
    config.default_model = Some("chat-v1".to_string());
    let (_, model) = make_openhuman_backend(&config).expect("factory should succeed");
    assert_eq!(model, "chat-v1");
}

#[test]
fn make_openhuman_backend_keeps_reasoning_quick() {
    let mut config = Config::default();
    config.default_model = Some("reasoning-quick-v1".to_string());
    let (_, model) = make_openhuman_backend(&config).expect("factory should succeed");
    assert_eq!(model, "reasoning-quick-v1");
}

// ── BYOK fail-closed tests ────────────────────────────────────────────────────

#[test]
fn byok_intent_no_primary_no_matching_entry_returns_sentinel() {
    // No primary_cloud set, inference_url points at a non-openhuman host with
    // no matching cloud_providers entry → must return the fail-closed sentinel.
    let mut config = Config::default();
    config.inference_url = Some("https://custom-api.example.com/v1".to_string());
    assert_eq!(
        provider_for_role("reasoning", &config),
        BYOK_INCOMPLETE_SENTINEL
    );
}

#[test]
fn byok_intent_with_matching_entry_resolves_correctly() {
    // Matching cloud_providers entry exists → legacy lookup succeeds; no sentinel.
    let mut custom = openai_entry("p_custom", "custom");
    custom.endpoint = "https://custom-api.example.com/v1".to_string();

    let mut config = config_with_providers(vec![custom]);
    config.inference_url = Some("https://custom-api.example.com/v1".to_string());

    // Legacy URL matches the custom entry → "custom:gpt-4o"
    assert_eq!(provider_for_role("reasoning", &config), "custom:gpt-4o");
}

#[test]
fn openhuman_inference_url_never_triggers_sentinel() {
    // inference_url pointing at the managed backend is not BYOK intent.
    let mut config = Config::default();
    config.inference_url = Some("https://api.openhuman.ai/v1".to_string());
    assert_eq!(provider_for_role("reasoning", &config), "openhuman");
}

#[test]
fn explicit_workload_route_bypasses_byok_sentinel() {
    // A per-role provider route set explicitly always wins over the BYOK check.
    let mut config = Config::default();
    config.inference_url = Some("https://custom-api.example.com/v1".to_string());
    config.reasoning_provider = Some("openhuman".to_string());
    // Explicit "openhuman" route → goes straight to backend, no sentinel.
    assert_eq!(provider_for_role("reasoning", &config), "openhuman");
}

#[test]
fn byok_sentinel_makes_provider_creation_error_with_clear_message() {
    let mut config = Config::default();
    config.inference_url = Some("https://custom-api.example.com/v1".to_string());

    // Use match instead of unwrap_err(): Box<dyn Provider> doesn't impl Debug.
    let msg = match create_chat_provider_from_string("reasoning", BYOK_INCOMPLETE_SENTINEL, &config)
    {
        Ok(_) => panic!("sentinel must produce an error, not a provider"),
        Err(e) => e.to_string(),
    };
    assert!(
        msg.contains("BYOK_INCOMPLETE"),
        "error must name BYOK_INCOMPLETE; got: {msg}"
    );
    assert!(
        msg.contains("custom-api.example.com"),
        "error must include the configured inference_url; got: {msg}"
    );
}

#[test]
fn byok_sentinel_error_mentions_configuration_action() {
    // The error message must tell the user how to fix the issue.
    let mut config = Config::default();
    config.inference_url = Some("https://byok.example.com/v1".to_string());

    // Use match instead of unwrap_err(): Box<dyn Provider> doesn't impl Debug.
    let msg = match create_chat_provider_from_string("chat", BYOK_INCOMPLETE_SENTINEL, &config) {
        Ok(_) => panic!("sentinel must produce an error"),
        Err(e) => e.to_string(),
    };
    // Must mention adding a cloud_providers entry or clearing inference_url.
    assert!(
        msg.contains("cloud_providers") || msg.contains("inference_url"),
        "error must suggest a remediation; got: {msg}"
    );
}

// ── BYOK workload inheritance tests ──────────────────────────────────────────

#[test]
fn byok_fallback_agentic_always_uses_managed_backend() {
    // The agentic role is excluded from BYOK inheritance: it uses managed-backend
    // tier models (agentic-v1) and handles hint:agentic routing directives.
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    // agentic_provider is unset and chat BYOK is configured → agentic must
    // still resolve to the managed backend, NOT inherit from chat BYOK.
    let result = provider_for_role("agentic", &config);
    assert_eq!(
        result, "openhuman",
        "agentic role must always resolve to managed backend regardless of BYOK config"
    );
}

#[test]
fn byok_fallback_inherits_chat_provider_for_unset_coding() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    // coding_provider is unset → should inherit chat BYOK
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openai:gpt-4o",
        "unset coding must inherit chat BYOK"
    );
    assert_ne!(result, "openhuman");
}

#[test]
fn byok_fallback_inherits_reasoning_when_chat_unset() {
    let mut config = Config::default();
    config
        .cloud_providers
        .push(anthropic_entry("p_ant", "anthropic"));
    config.reasoning_provider = Some("anthropic:claude-opus-4-7".to_string());
    // coding_provider is unset, chat_provider is unset → should inherit reasoning BYOK
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "anthropic:claude-opus-4-7",
        "unset coding must inherit reasoning BYOK when chat is unset"
    );
}

#[test]
fn byok_fallback_respects_priority_order() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config
        .cloud_providers
        .push(anthropic_entry("p_ant", "anthropic"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.reasoning_provider = Some("anthropic:claude-opus-4-7".to_string());
    // chat wins (higher priority) for unset coding
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openai:gpt-4o",
        "chat_provider must win over reasoning_provider in priority"
    );
}

#[test]
fn byok_fallback_skips_local_ollama() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:llama3.1".to_string());
    // Ollama is local — must NOT be inherited for non-agentic roles either
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openhuman",
        "local ollama must not be inherited as BYOK fallback"
    );
}

#[test]
fn byok_fallback_skips_local_lmstudio() {
    let mut config = Config::default();
    config.chat_provider = Some("lmstudio:google/gemma-4-e4b".to_string());
    // LM Studio is local — must NOT be inherited; fall through to openhuman
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openhuman",
        "local lmstudio must not be inherited as BYOK fallback"
    );
}

#[test]
fn byok_fallback_skips_openhuman_sentinel() {
    let mut config = Config::default();
    config.chat_provider = Some("openhuman".to_string());
    // "openhuman" is the managed backend sentinel, not BYOK
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openhuman",
        "openhuman sentinel in chat must not be treated as BYOK"
    );
}

#[test]
fn byok_fallback_skips_cloud_sentinel() {
    let mut config = Config::default();
    config.chat_provider = Some("cloud".to_string());
    // "cloud" means "use primary" — not BYOK
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openhuman",
        "cloud sentinel in chat must not be treated as BYOK"
    );
}

#[test]
fn byok_fallback_no_byok_configured() {
    // All workload routes unset → falls through to managed backend unchanged
    let config = Config::default();
    assert_eq!(
        provider_for_role("coding", &config),
        "openhuman",
        "no BYOK configured must fall through to openhuman for coding"
    );
    assert_eq!(
        provider_for_role("agentic", &config),
        "openhuman",
        "no BYOK configured must fall through to openhuman for agentic"
    );
}

#[test]
fn byok_fallback_explicit_agentic_overrides_chat_byok() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config
        .cloud_providers
        .push(anthropic_entry("p_ant", "anthropic"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.agentic_provider = Some("anthropic:claude-haiku-4-5".to_string());
    // Explicit agentic setting wins over BYOK inheritance
    let result = provider_for_role("agentic", &config);
    assert_eq!(
        result, "anthropic:claude-haiku-4-5",
        "explicit agentic_provider must win over inherited BYOK"
    );
}

#[test]
fn byok_fallback_explicit_openhuman_agentic_overrides_chat_byok() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.agentic_provider = Some("openhuman".to_string());
    // Explicit "openhuman" in agentic wins — user made a deliberate choice
    let result = provider_for_role("agentic", &config);
    assert_eq!(
        result, "openhuman",
        "explicit openhuman in agentic must not be overridden by BYOK inheritance"
    );
}

#[test]
fn byok_fallback_all_workloads_set_independently() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config
        .cloud_providers
        .push(anthropic_entry("p_ant", "anthropic"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.reasoning_provider = Some("anthropic:claude-opus-4-7".to_string());
    config.agentic_provider = Some("anthropic:claude-haiku-4-5".to_string());
    config.coding_provider = Some("openai:gpt-4o-mini".to_string());
    assert_eq!(provider_for_role("chat", &config), "openai:gpt-4o");
    assert_eq!(
        provider_for_role("reasoning", &config),
        "anthropic:claude-opus-4-7"
    );
    assert_eq!(
        provider_for_role("agentic", &config),
        "anthropic:claude-haiku-4-5"
    );
    assert_eq!(provider_for_role("coding", &config), "openai:gpt-4o-mini");
}

#[test]
fn byok_fallback_empty_string_treated_as_unset() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.coding_provider = Some(String::new()); // empty string = unset
                                                  // Empty string must be treated as unset → coding inherits chat BYOK
    let result = provider_for_role("coding", &config);
    assert_eq!(
        result, "openai:gpt-4o",
        "empty coding_provider must be treated as unset and inherit chat BYOK"
    );
    // agentic is excluded from BYOK inheritance regardless
    config.agentic_provider = Some(String::new());
    let agentic_result = provider_for_role("agentic", &config);
    assert_eq!(
        agentic_result, "openhuman",
        "empty agentic_provider must stay on managed backend even when chat BYOK is configured"
    );
}

// ── resolve_byok_fallback_provider_string direct tests ───────────────────────

#[test]
fn resolve_byok_fallback_returns_none_when_no_byok() {
    let config = Config::default();
    assert!(
        resolve_byok_fallback_provider_string(&config).is_none(),
        "all routes empty must return None"
    );
}

#[test]
fn resolve_byok_fallback_returns_none_for_local_only() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:llama3.1".to_string());
    config.reasoning_provider = Some("lmstudio:google/gemma".to_string());
    assert!(
        resolve_byok_fallback_provider_string(&config).is_none(),
        "only local providers must return None"
    );
}

#[test]
fn resolve_byok_fallback_returns_some_for_openai() {
    let mut config = Config::default();
    config.chat_provider = Some("openai:gpt-4o".to_string());
    let result = resolve_byok_fallback_provider_string(&config);
    assert_eq!(result, Some("openai:gpt-4o".to_string()));
}

#[test]
fn resolve_byok_fallback_returns_some_for_anthropic() {
    let mut config = Config::default();
    config.reasoning_provider = Some("anthropic:claude-sonnet-4-6".to_string());
    let result = resolve_byok_fallback_provider_string(&config);
    assert_eq!(result, Some("anthropic:claude-sonnet-4-6".to_string()));
}

#[test]
fn resolve_byok_fallback_skips_empty_and_finds_next() {
    let mut config = Config::default();
    config.chat_provider = Some(String::new()); // empty — skipped
    config.reasoning_provider = Some("anthropic:claude-opus-4-7".to_string());
    let result = resolve_byok_fallback_provider_string(&config);
    assert_eq!(result, Some("anthropic:claude-opus-4-7".to_string()));
}

#[test]
fn byok_fallback_background_workloads_never_inherit() {
    // Background workloads (memory, embeddings, heartbeat, learning, subconscious)
    // must stay on the managed backend even when chat BYOK is configured.
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    for role in &[
        "memory",
        "embeddings",
        "heartbeat",
        "learning",
        "subconscious",
    ] {
        let result = provider_for_role(role, &config);
        assert_eq!(
            result, "openhuman",
            "background workload '{}' must not inherit chat BYOK",
            role
        );
    }
}

#[tokio::test]
#[ignore = "requires live LM Studio on localhost:1234"]
async fn live_lmstudio_provider_streams_thinking_and_text() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://127.0.0.1:1234/v1".to_string());
    let model = discover_live_lmstudio_model()
        .await
        .expect("discover live lmstudio model");
    let provider_string = format!("lmstudio:{model}");
    let (provider, resolved_model) =
        create_local_chat_provider_from_string(&provider_string, &config).expect("build provider");

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let messages = vec![ChatMessage::user(
        "Think briefly, then reply with exactly LMSTUDIO_LIVE_OK.",
    )];
    let response = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: Some(&tx),
            },
            &resolved_model,
            0.0,
        )
        .await
        .expect("live lmstudio chat");
    drop(tx);

    let mut saw_thinking = false;
    let mut streamed_text = String::new();
    while let Some(delta) = rx.recv().await {
        match delta {
            ProviderDelta::ThinkingDelta { delta } => {
                if !delta.trim().is_empty() {
                    saw_thinking = true;
                }
            }
            ProviderDelta::TextDelta { delta } => streamed_text.push_str(&delta),
            ProviderDelta::ToolCallStart { .. } | ProviderDelta::ToolCallArgsDelta { .. } => {}
        }
    }

    assert!(
        saw_thinking,
        "LM Studio should emit reasoning/thinking deltas through the compatible provider path"
    );
    assert!(
        response.text_or_empty().contains("LMSTUDIO_LIVE_OK"),
        "unexpected final response: {:?}",
        response.text
    );
    assert!(
        streamed_text.contains("LMSTUDIO_LIVE_OK"),
        "streamed text never surfaced the final answer: {streamed_text}"
    );
}

#[tokio::test]
#[ignore = "requires live Ollama on localhost:11434"]
async fn live_ollama_provider_streams_text() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://127.0.0.1:11434".to_string());
    let model = discover_live_ollama_model()
        .await
        .expect("discover live ollama model");
    let provider_string = format!("ollama:{model}");
    let (provider, resolved_model) =
        create_local_chat_provider_from_string(&provider_string, &config).expect("build provider");

    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let messages = vec![ChatMessage::user("Reply with exactly OLLAMA_LIVE_OK.")];
    let response = provider
        .chat(
            ChatRequest {
                messages: &messages,
                tools: None,
                stream: Some(&tx),
            },
            &resolved_model,
            0.0,
        )
        .await
        .expect("live ollama chat");
    drop(tx);

    let mut streamed_text = String::new();
    while let Some(delta) = rx.recv().await {
        if let ProviderDelta::TextDelta { delta } = delta {
            streamed_text.push_str(&delta);
        }
    }

    assert!(
        response.text_or_empty().contains("OLLAMA_LIVE_OK"),
        "unexpected final response: {:?}",
        response.text
    );
    assert!(
        streamed_text.contains("OLLAMA_LIVE_OK"),
        "streamed text never surfaced the final answer: {streamed_text}"
    );
}
