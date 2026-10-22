use super::*;
use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::config::Config;
use crate::inference::provider::factory::resolves_to_managed_backend;

fn byok_openai_compatible_entry() -> CloudProviderCreds {
    CloudProviderCreds {
        id: "p_my-openai".to_string(),
        slug: "my-openai".to_string(),
        label: "My OpenAI".to_string(),
        endpoint: "https://inference.example.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        ..Default::default()
    }
}

fn config_with_byok_provider() -> Config {
    let mut config = Config::default();
    config.cloud_providers = vec![byok_openai_compatible_entry()];
    config
}

/// #6938: the Chat UI model picker records its selection in
/// `config.default_model` (the web-chat turn path stores the per-turn
/// `model_override` there), but role resolution never consulted it — every
/// turn with an unset `chat_provider` silently fell back to the managed
/// backend and 401'd as "session expired" on local-only profiles.
#[test]
fn default_model_with_explicit_provider_routes_chat_instead_of_managed() {
    let mut config = config_with_byok_provider();
    config.default_model = Some("my-openai:gpt-4o".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "my-openai:gpt-4o",
        "a picked provider in default_model must route the chat turn"
    );
    assert!(
        !resolves_to_managed_backend("chat", &config),
        "the chat turn must not resolve to the managed backend when a provider was picked"
    );
}

/// The default-model route covers the chat-tier roles, whose turns the
/// picker is meant to influence.
#[test]
fn default_model_route_applies_to_chat_tier_roles() {
    let mut config = config_with_byok_provider();
    config.default_model = Some("my-openai:gpt-4o".to_string());

    for role in ["chat", "reasoning", "agentic", "coding"] {
        assert_eq!(
            provider_for_role(role, &config),
            "my-openai:gpt-4o",
            "`{role}` must honour the picked provider in default_model"
        );
    }
}

/// A chat-model pick says nothing about which model should do vision or
/// summarization — the background specialist roles keep their managed
/// fallback.
#[test]
fn default_model_route_does_not_touch_background_roles() {
    let mut config = config_with_byok_provider();
    config.default_model = Some("my-openai:gpt-4o".to_string());

    for role in [
        "vision",
        "memory",
        "summarization",
        "embeddings",
        "learning",
    ] {
        assert_eq!(
            provider_for_role(role, &config),
            "openhuman",
            "`{role}` must keep the managed fallback despite the default_model pick"
        );
    }
}

/// Everything `default_model` can hold that is *not* an explicit provider
/// route must keep the managed fallback: tier hints, the AI settings row's
/// managed catalog pin (`openrouter/...`), bare model ids, unknown slugs,
/// and the managed provider itself.
#[test]
fn default_model_without_explicit_provider_keeps_managed_fallback() {
    let mut config = config_with_byok_provider();
    for default_model in [
        "hint:chat",
        "openrouter/deepseek/deepseek-v3",
        "openrouter/deepseek/deepseek-v3:free",
        "gpt-4o",
        "",
        "nosuchprovider:some-model",
        "openhuman:some-model",
        "my-openai:",
        ":gpt-4o",
    ] {
        config.default_model = Some(default_model.to_string());
        assert_eq!(
            provider_for_role("chat", &config),
            "openhuman",
            "default_model={default_model:?} must not reroute the chat turn"
        );
        assert!(
            resolves_to_managed_backend("chat", &config),
            "default_model={default_model:?} must keep the managed backend"
        );
    }
}

/// A local runtime pick (`ollama:...`) routes the chat turn locally instead
/// of the managed backend — same mechanism, no cloud provider involved.
#[test]
fn default_model_with_local_provider_routes_chat_locally() {
    let mut config = Config::default();
    config.default_model = Some("ollama:llama3".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "ollama:llama3",
        "a picked local model must route the chat turn locally"
    );
    assert!(
        !resolves_to_managed_backend("chat", &config),
        "a picked local model must not resolve to the managed backend"
    );
}

/// An explicitly configured role route still wins over the default model —
/// the #6109 "each route stands alone" invariant is preserved.
#[test]
fn explicit_role_route_still_wins_over_default_model() {
    let mut config = config_with_byok_provider();
    config.chat_provider = Some("anthropic:claude-x".to_string());
    config.default_model = Some("my-openai:gpt-4o".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "anthropic:claude-x",
        "the explicitly configured chat_provider must win over default_model"
    );
}

/// The #6109 regression in the other direction: a configured sibling route
/// is used for its own role while the default model covers the unset ones —
/// no cross-contamination either way.
#[test]
fn default_model_does_not_disturb_configured_sibling_routes() {
    let mut config = config_with_byok_provider();
    config.coding_provider = Some("my-openai:codestral".to_string());
    config.default_model = Some("my-openai:gpt-4o".to_string());

    assert_eq!(
        provider_for_role("coding", &config),
        "my-openai:codestral",
        "the configured coding route must be honoured as-is"
    );
    assert_eq!(
        provider_for_role("chat", &config),
        "my-openai:gpt-4o",
        "the unset chat route falls back to the default_model pick, not the sibling's route"
    );
    assert_eq!(
        provider_for_role("reasoning", &config),
        "my-openai:gpt-4o",
        "the unset reasoning route falls back to the default_model pick, not the sibling's route"
    );
}

/// tinysweeper review on #6996: a bare `claude_agent_sdk` default names the
/// provider with no `:model` part. The old `split_once(':')?` rejected it
/// before the explicit `CLAUDE_AGENT_SDK_PROVIDER` check could run, silently
/// falling back to the managed backend.
#[test]
fn default_model_with_bare_claude_agent_sdk_routes_chat() {
    let mut config = Config::default();
    config.default_model = Some("claude_agent_sdk".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "claude_agent_sdk",
        "a bare claude_agent_sdk pick must route the chat turn"
    );
    assert!(
        !resolves_to_managed_backend("chat", &config),
        "a bare claude_agent_sdk pick must not resolve to the managed backend"
    );
}

/// Bare local provider names (`ollama` with no `:model` part) keep the managed
/// fallback: the local runtime constructor rejects an empty model
/// (`empty_model_err` in `local_runtime.rs`), so routing there would only
/// trade the managed backend for a build-time error (CodeRabbit review on
/// #6996).
#[test]
fn default_model_with_bare_local_provider_keeps_managed_fallback() {
    let mut config = Config::default();
    config.default_model = Some("ollama".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "openhuman",
        "a bare ollama pick has no model id and must keep the managed fallback"
    );
    assert!(
        resolves_to_managed_backend("chat", &config),
        "a bare ollama pick must resolve to the managed backend"
    );
}

/// A bare cloud slug (`my-openai` with no `:model` part) is not constructible
/// either — the cloud-slug path requires the `<slug>:<model>` form — so it
/// keeps the managed fallback as well.
#[test]
fn default_model_with_bare_cloud_slug_keeps_managed_fallback() {
    let mut config = config_with_byok_provider();
    config.default_model = Some("my-openai".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "openhuman",
        "a bare cloud slug has no model id and must keep the managed fallback"
    );
    assert!(
        resolves_to_managed_backend("chat", &config),
        "a bare cloud slug must resolve to the managed backend"
    );
}

/// Bare `openai` is deliberately NOT rerouted: the `openai` slug names the
/// cloud provider elsewhere in the factory, so it keeps the managed fallback
/// rather than being misread as a local runtime.
#[test]
fn default_model_with_bare_openai_keeps_managed_fallback() {
    let mut config = Config::default();
    config.default_model = Some("openai".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "openhuman",
        "bare openai must keep the managed fallback"
    );
    assert!(
        resolves_to_managed_backend("chat", &config),
        "bare openai must resolve to the managed backend"
    );
}

fn oci_entry(auth_mode: crate::config::schema::cloud_providers::OciAuthMode) -> CloudProviderCreds {
    CloudProviderCreds {
        id: "p_oci".to_string(),
        slug: "oci".to_string(),
        label: "OCI Generative AI".to_string(),
        auth_style: AuthStyle::Oci,
        oci: Some(
            crate::config::schema::cloud_providers::OciProviderSettings {
                auth_mode,
                ..Default::default()
            },
        ),
        ..Default::default()
    }
}

/// OCI profile, session-token and principal modes keep no key in the
/// credential store, so the billing gate must not require one for them.
#[test]
fn keyless_oci_routes_count_as_usable_credentials() {
    use crate::config::schema::cloud_providers::OciAuthMode;
    for mode in [
        OciAuthMode::ConfigProfile,
        OciAuthMode::SessionToken,
        OciAuthMode::ResourcePrincipal,
        OciAuthMode::InstancePrincipal,
        OciAuthMode::OkeWorkloadIdentity,
    ] {
        let mut config = Config::default();
        config.cloud_providers = vec![oci_entry(mode)];
        assert!(
            route_has_usable_credentials("oci:openai.gpt-5.5", &config),
            "{mode:?}"
        );
    }
}
