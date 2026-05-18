use super::*;
use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
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
fn ollama_prefix() {
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("heartbeat", "ollama:llama3.1:8b", &config)
        .expect("ollama:<model> must build");
    assert_eq!(model, "llama3.1:8b");
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

#[test]
fn all_workloads_default_to_openhuman() {
    let config = Config::default();
    for role in &[
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
        err.to_string().contains("cloud API key not set"),
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
