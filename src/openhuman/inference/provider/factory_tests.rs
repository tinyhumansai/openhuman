use super::*;
use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::openhuman::config::Config;
use crate::openhuman::credentials::AuthService;
use crate::openhuman::inference::provider::traits::{ChatMessage, ChatRequest, ProviderDelta};
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

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
fn ollama_provider_opts_out_of_native_tool_calling() {
    // Sub-issue 3 of #3098: Ollama's OpenAI-compat endpoint returns HTTP 400
    // for many models when a `tools` array is sent (the existing detection
    // path matches "unsupported parameter: tools"). The retry logic strips
    // tools entirely, which silently breaks any skill or workflow that
    // depends on tool calls. The factory must build the Ollama provider
    // with native tool calling disabled so the agent harness uses the
    // prompt-guided text format from the first request.
    let config = Config::default();
    let (provider, _model) = create_chat_provider_from_string("chat", "ollama:llama3.2", &config)
        .expect("ollama:<model> must build");
    let caps = provider.capabilities();
    assert!(
        !caps.native_tool_calling,
        "ollama provider must report native_tool_calling=false so the agent harness emits prompt-guided tool specs instead of an OpenAI-style `tools` array"
    );
    assert!(
        !caps.vision,
        "local Ollama-compatible providers stay fail-closed for vision until the configured model proves image support"
    );
}

#[test]
fn lmstudio_provider_defaults_to_prompt_guided_tools() {
    // All local providers (Ollama, LM Studio, MLX, local-openai) default to
    // prompt-guided tool dispatch (#3246). This prevents HTTP 400 errors
    // from models that don't support the native `tools` parameter. Users
    // can override via `config.agent.tool_dispatcher = "native"` if their
    // model supports it.
    let mut config = Config::default();
    config.local_ai.base_url = Some("http://127.0.0.1:1234".to_string());
    let (provider, _model) =
        create_chat_provider_from_string("chat", "lmstudio:google/gemma-4-e4b", &config)
            .expect("lmstudio:<model> must build");
    let caps = provider.capabilities();
    assert!(
        !caps.native_tool_calling,
        "lmstudio provider must default to native_tool_calling=false (conservative local dispatch)"
    );
    assert!(
        !caps.vision,
        "local LM Studio-compatible providers stay fail-closed for vision until the configured model proves image support"
    );
}

// Note: a BYOK-cloud regression test (e.g. `openai:gpt-4o` keeps
// native_tool_calling=true) would need an `AuthService` with the slug's API
// key seeded. The unit test
// `with_native_tool_calling_true_preserves_default` in compatible_tests.rs
// already pins that the builder leaves the default in place when not
// called, which is what every non-Ollama factory path relies on.

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

// Regression (#hint-routing): on the managed OpenHuman backend, a specialised
// workload role must resolve to its dedicated tier — NOT collapse to
// `default_model` (which defaults to `chat-v1`). Before the fix,
// `make_openhuman_backend` only special-cased `vision`, so `hint = "coding"`
// sub-agents (code_executor, skill_creator, tool_maker) silently ran on
// `chat-v1` instead of `coding-v1`, and likewise for `agentic`/`reasoning`.
// (`summarization`/`memory` resolve their tier separately from
// `memory_tree.cloud_llm_model` — see
// `managed_backend_summarization_role_resolves_summarization_tier`.) This drives
// `make_openhuman_backend` directly via the explicit `"openhuman"` provider
// string.
#[test]
fn managed_backend_pins_specialised_role_to_tier() {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CODING_V1, MODEL_REASONING_V1, MODEL_VISION_V1,
    };
    // default_model is chat-v1 — the value the buggy path would have leaked.
    let config = Config::default();
    assert_eq!(config.default_model.as_deref(), Some("chat-v1"));

    for (role, expected_tier) in &[
        ("reasoning", MODEL_REASONING_V1),
        ("agentic", MODEL_AGENTIC_V1),
        ("burst", MODEL_BURST_V1),
        ("coding", MODEL_CODING_V1),
        ("vision", MODEL_VISION_V1),
    ] {
        let (_, model) = create_chat_provider_from_string(role, "openhuman", &config)
            .expect("managed backend must build");
        assert_eq!(
            model, *expected_tier,
            "role={role} must pin to {expected_tier} on the managed backend, got {model}"
        );
    }
}

// The managed `summarization`/`memory` role is fixed at `summarization-v1` (via
// `summarization_tier_model`), independent of both `config.default_model` and
// `memory_tree.cloud_llm_model`. This is what makes EVERY managed summarization
// caller — memory tree, chat-turn payload summarizer, meeting summaries, and
// `hint = "summarization"` sub-agents — reach the dedicated `summarization-v1`
// tier without each caller pre-routing `default_model`.
#[test]
fn managed_backend_summarization_role_resolves_summarization_tier() {
    // Default config: cloud_llm_model defaults to summarization-v1.
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("summarization", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "summarization-v1");

    // `memory` is an alias of `summarization` (both → memory_provider).
    let (_, model) = create_chat_provider_from_string("memory", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "summarization-v1");
}

// `default_model` does NOT drive the summarization tier any more — only
// `memory_tree.cloud_llm_model` does. A stray `default_model` must not leak in.
#[test]
fn managed_backend_summarization_ignores_default_model() {
    let mut config = Config::default();
    config.default_model = Some("reasoning-v1".to_string());
    let (_, model) = create_chat_provider_from_string("summarization", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "summarization-v1");
}

// The managed summarization tier is LOCKED to `summarization-v1` — the
// (deprecated, inert) `memory_tree.cloud_llm_model` must not change it, whether
// set to another known tier or a custom string. Users who want a different model
// run summarization on a BYOK/local `memory_provider` instead.
#[test]
fn managed_backend_summarization_ignores_cloud_llm_model_override() {
    let mut config = Config::default();
    config.memory_tree.cloud_llm_model = Some("chat-v1".to_string());
    let (_, model) = create_chat_provider_from_string("summarization", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "summarization-v1");

    config.memory_tree.cloud_llm_model = Some("custom-summary-model".to_string());
    let (_, model) = create_chat_provider_from_string("summarization", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "summarization-v1");
}

// End-to-end of the sub-agent path: the subagent runner resolves a
// `ModelSpec::Hint(workload)` by calling `create_chat_provider(workload, cfg)`.
// With a default config (every per-workload provider unset → managed backend),
// each shipped hint must still reach its tier. This is the exact call the
// `code_executor` agent (`hint = "coding"`) makes when it spawns.
#[test]
fn subagent_hint_resolves_to_tier_on_managed_backend() {
    use crate::openhuman::config::{
        MODEL_AGENTIC_V1, MODEL_BURST_V1, MODEL_CODING_V1, MODEL_REASONING_V1,
    };
    let config = Config::default();
    for (hint, expected_tier) in &[
        ("coding", MODEL_CODING_V1),
        ("agentic", MODEL_AGENTIC_V1),
        ("burst", MODEL_BURST_V1),
        ("reasoning", MODEL_REASONING_V1),
    ] {
        let (_, model) =
            create_chat_provider(hint, &config).expect("create_chat_provider must succeed");
        assert_eq!(
            model, *expected_tier,
            "hint={hint} sub-agent must run on {expected_tier}, got {model}"
        );
    }
}

// The generic `chat` role must keep inheriting `default_model` — the front-line
// chat turn and legacy `default_model = "reasoning-v1"` installs deliberately
// fall through to the `chat` role (see the session builder), so pinning `chat`
// would regress them.
#[test]
fn managed_backend_chat_role_inherits_default_model() {
    // Default (chat-v1).
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("chat", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "chat-v1");

    // Legacy literal: a user pinned `default_model = "reasoning-v1"` must still
    // get reasoning-v1 for the chat-role front-line turn.
    let mut config = Config::default();
    config.default_model = Some("reasoning-v1".to_string());
    let (_, model) = create_chat_provider_from_string("chat", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "reasoning-v1");
}

// The tier pin only governs the *managed* backend. A user who routes the coding
// workload to their own BYOK provider must still get that provider's model —
// `provider_for_role` resolves the per-workload route before the managed path.
#[test]
fn coding_workload_byok_route_wins_over_managed_pin() {
    let mut config = Config::default();
    config
        .cloud_providers
        .push(anthropic_entry("p_ant", "anthropic"));
    config.coding_provider = Some("anthropic:claude-sonnet-4-6".to_string());
    let (_, model) =
        create_chat_provider("coding", &config).expect("create_chat_provider must succeed");
    assert_eq!(model, "claude-sonnet-4-6");
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
fn cloud_provider_with_no_model_and_no_default_rejected() {
    // TAURI-RUST-4NM — nvidia-nim (and others) reject `model=""` with
    // "model field is required". The factory must catch this up-front with
    // a clear, actionable message instead of leaking an empty model to the API.
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_nim".to_string(),
        slug: "nvidia-nim".to_string(),
        label: "NVIDIA NIM".to_string(),
        endpoint: "https://integrate.api.nvidia.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: None, // no fallback model configured
        ..Default::default()
    });

    let err = match create_chat_provider_from_string("reasoning", "nvidia-nim:", &config) {
        Ok(_) => panic!("empty model must fail"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("no model configured"),
        "expected 'no model configured' in error, got: {msg}"
    );
    assert!(
        msg.contains("nvidia-nim"),
        "error must name the slug; got: {msg}"
    );

    // TAURI-RUST-GKV coupling — the SAME bail body that floods Sentry must:
    //   (a) still contain the classifier anchor const, and
    //   (b) be recognised by the shared config-rejection classifier
    //       (which both demotes the Sentry event AND drives the actionable
    //       user-facing copy in `classify_inference_error`).
    // If the bail wording drifts off the anchor, (a) fails; if the
    // classifier phrase drifts, (b) fails — CI catches either direction, so
    // the demotion can never silently regress into an error flood.
    assert!(
        msg.contains(super::NO_MODEL_CONFIGURED_ANCHOR),
        "bail body must contain NO_MODEL_CONFIGURED_ANCHOR; got: {msg}"
    );
    assert!(
        crate::openhuman::inference::provider::is_provider_config_rejection_message(&msg),
        "empty-model bail must classify as provider config-rejection: {msg}"
    );
}

#[test]
fn cloud_provider_default_model_used_when_model_part_is_empty() {
    // When provider string is "nvidia-nim:" (empty model) but the entry
    // has a default_model, the factory must use the default — not error.
    let mut config = Config::default();
    config.cloud_providers.push(CloudProviderCreds {
        id: "p_nim".to_string(),
        slug: "nvidia-nim".to_string(),
        label: "NVIDIA NIM".to_string(),
        endpoint: "https://integrate.api.nvidia.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("meta/llama-3.1-8b-instruct".to_string()),
        ..Default::default()
    });

    let (_, model) = create_chat_provider_from_string("reasoning", "nvidia-nim:", &config)
        .expect("empty model with default_model must succeed");
    assert_eq!(model, "meta/llama-3.1-8b-instruct");
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
    assert!(
        provider.capabilities().vision,
        "cloud OpenAI-compatible providers must advertise vision so reasoning attachment turns reach the provider"
    );

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
        "burst-v1",
        "coding-v1",
        "reasoning-quick-v1",
        "summarization-v1",
        "vision-v1",
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
    assert!(is_known_openhuman_tier("hint:burst"));
    assert!(is_known_openhuman_tier("hint:coding"));
    assert!(is_known_openhuman_tier("hint:summarization"));
    assert!(is_known_openhuman_tier("hint:vision"));
}

// `hint:burst` is accepted by `is_known_openhuman_tier`, so it must also be
// translated to `burst-v1` by the managed backend — otherwise a saved
// `default_model = "hint:burst"` would be forwarded literally and 400.
#[test]
fn managed_backend_translates_hint_burst_to_burst_tier() {
    let mut config = Config::default();
    config.default_model = Some("hint:burst".to_string());
    let (_, model) = create_chat_provider_from_string("chat", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, crate::openhuman::config::MODEL_BURST_V1);
}

#[test]
fn invalid_models_fail() {
    assert!(!is_known_openhuman_tier("deepseek-v4-pro"));
    assert!(!is_known_openhuman_tier("claude-opus-4-7"));
    assert!(!is_known_openhuman_tier("gpt-4o"));
    assert!(!is_known_openhuman_tier(""));
    assert!(!is_known_openhuman_tier("reasoning-v2"));
    // Unrecognized `hint:*` values must NOT be accepted — the factory only
    // translates the known hints above, so any other `hint:*` string would
    // otherwise be forwarded to the backend and rejected with HTTP 400.
    assert!(!is_known_openhuman_tier("hint:garbage"));
    assert!(!is_known_openhuman_tier("hint:reasoning-quick"));
    assert!(!is_known_openhuman_tier("hint:"));
}

// ── is_raw_passthrough_model ─────────────────────────────────────────────────
// Raw/BYOK model ids (non-empty, non-tier, non-`hint:*`) must be recognized as
// passthrough so they reach provider construction verbatim (issue #4598).
#[test]
fn raw_passthrough_model_detects_byok_ids() {
    assert!(is_raw_passthrough_model("claude-opus-4"));
    assert!(is_raw_passthrough_model("deepseek-v4-pro"));
    assert!(is_raw_passthrough_model("gpt-4o"));
    assert!(is_raw_passthrough_model("reasoning-v2"));
    // Surrounding whitespace is trimmed before the classification.
    assert!(is_raw_passthrough_model("  claude-opus-4  "));
}

#[test]
fn raw_passthrough_model_excludes_tiers_hints_and_empty() {
    // Managed tiers are resolved, never forwarded raw.
    assert!(!is_raw_passthrough_model("reasoning-v1"));
    assert!(!is_raw_passthrough_model("chat-v1"));
    assert!(!is_raw_passthrough_model("summarization-v1"));
    // Every `hint:*` alias (known or not) stays on the hint-resolution path.
    assert!(!is_raw_passthrough_model("hint:reasoning"));
    assert!(!is_raw_passthrough_model("hint:garbage"));
    // Empty / whitespace-only ids are not passthrough — they fall back to default.
    assert!(!is_raw_passthrough_model(""));
    assert!(!is_raw_passthrough_model("   "));
}

// End-to-end through the public factory string entry: a raw BYOK model pinned in
// `default_model` reaches provider construction verbatim on the managed backend,
// while a known tier and a `hint:*` alias keep their existing resolution.
#[test]
fn managed_backend_passthrough_via_create_chat_provider_from_string() {
    let mut config = Config::default();
    config.default_model = Some("claude-opus-4".to_string());
    let (_, model) = create_chat_provider_from_string("chat", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "claude-opus-4");

    // Managed tier resolution is unchanged.
    config.default_model = Some("chat-v1".to_string());
    let (_, model) = create_chat_provider_from_string("chat", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, "chat-v1");

    // `hint:*` resolution is unchanged.
    config.default_model = Some("hint:reasoning".to_string());
    let (_, model) = create_chat_provider_from_string("chat", "openhuman", &config)
        .expect("managed backend must build");
    assert_eq!(model, crate::openhuman::config::MODEL_REASONING_V1);
}

// ── oh_tier_supports_vision ──────────────────────────────────────────────────────

#[test]
fn reasoning_is_the_vision_capable_managed_tier() {
    // `reasoning-v1` (and its hint form) is the one vision-capable managed tier.
    assert!(oh_tier_supports_vision("reasoning-v1"));
    assert!(oh_tier_supports_vision("hint:reasoning"));

    // Every other managed tier (and its hint form) is non-vision until confirmed
    // multimodal on the backend. Flip the corresponding arm in
    // `oh_tier_supports_vision` to enable one.
    for model in [
        "chat-v1",
        "agentic-v1",
        "burst-v1",
        "coding-v1",
        "reasoning-quick-v1",
        "summarization-v1",
        "hint:chat",
        "hint:agentic",
        "hint:burst",
        "hint:coding",
        "hint:summarization",
    ] {
        assert!(
            !oh_tier_supports_vision(model),
            "expected managed tier '{model}' to be non-vision"
        );
    }
}

#[test]
fn unknown_models_are_not_vision_capable() {
    assert!(!oh_tier_supports_vision("gpt-5"));
    assert!(!oh_tier_supports_vision("claude-opus-4-7"));
    assert!(!oh_tier_supports_vision(""));
}

#[test]
fn vision_tier_is_vision_capable() {
    // The dedicated multimodal tier (and its hint form) reports vision support,
    // so the turn engine's image gate accepts image turns for the vision
    // sub-agent — managed or BYOK (which resolves via this same alias).
    assert!(oh_tier_supports_vision("vision-v1"));
    assert!(oh_tier_supports_vision("hint:vision"));
}

#[test]
fn make_openhuman_backend_forwards_unknown_hint_verbatim() {
    // Unrecognised hint:* strings (e.g. hint:reaction for lightweight models)
    // must be forwarded to the backend unchanged. The backend is authoritative
    // over which hint values it accepts; the factory only translates the
    // canonical hints (reasoning/chat/agentic/coding/summarization).
    // `hint:summarization` became canonical when `summarization-v1` shipped
    // (PR #2690), so it is no longer a passthrough case.
    for hint in ["hint:reaction", "hint:garbage", "hint:lightweight"] {
        let mut config = Config::default();
        config.default_model = Some(hint.to_string());
        let (_, model) = make_openhuman_backend("chat", &config).expect("factory should succeed");
        assert_eq!(model, hint, "hint '{hint}' should pass through unchanged");
    }
}

#[test]
fn make_openhuman_backend_translates_summarization_hint() {
    let mut config = Config::default();
    config.default_model = Some("hint:summarization".to_string());
    let (_, model) = make_openhuman_backend("chat", &config).expect("factory should succeed");
    assert_eq!(model, crate::openhuman::config::MODEL_SUMMARIZATION_V1);
}

#[test]
fn managed_backend_pins_subconscious_role_to_chat_tier() {
    // Subconscious is pinned to chat-v1 on the managed backend via
    // `managed_tier_for_role`, *independent of* `default_model`. The cloud tick
    // routes through this role with `default_model` overwritten to the
    // "hint:subconscious" marker, so it must NOT inherit `default_model` (which
    // would forward the raw marker to the backend → HTTP 400).
    let mut config = Config::default();
    config.default_model = Some("hint:subconscious".to_string());
    let (_, model) =
        make_openhuman_backend("subconscious", &config).expect("factory should succeed");
    assert_eq!(model, crate::openhuman::config::MODEL_CHAT_V1);

    // Even with a heavy `default_model`, the subconscious role stays on chat-v1.
    config.default_model = Some("reasoning-v1".to_string());
    let (_, model) =
        make_openhuman_backend("subconscious", &config).expect("factory should succeed");
    assert_eq!(model, crate::openhuman::config::MODEL_CHAT_V1);
}

#[test]
fn create_chat_provider_subconscious_managed_resolves_chat_v1() {
    // End-to-end of the managed tick path: provider role `subconscious` with the
    // hint default_model, no BYOK subconscious_provider → managed backend, model
    // pinned to chat-v1 (no regression vs the pre-change chat-role behaviour).
    let mut config = Config::default();
    config.default_model = Some("hint:subconscious".to_string());
    let (_, model) =
        create_chat_provider("subconscious", &config).expect("create_chat_provider must succeed");
    assert_eq!(model, crate::openhuman::config::MODEL_CHAT_V1);
}

#[test]
fn create_chat_provider_subconscious_honours_byok_route() {
    // When the user pins a concrete cloud provider for the subconscious workload
    // in Settings → AI → Advanced, the factory builds that provider and returns
    // its exact model id.
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.subconscious_provider = Some("openai:gpt-4o-mini".to_string());
    let (_, model) =
        create_chat_provider("subconscious", &config).expect("create_chat_provider must succeed");
    assert_eq!(model, "gpt-4o-mini");
}

#[test]
fn provider_for_role_subconscious_override_respected() {
    let mut config = Config::default();
    config.subconscious_provider = Some("ollama:llama3.2:3b".to_string());
    assert_eq!(
        provider_for_role("subconscious", &config),
        "ollama:llama3.2:3b"
    );
    // Unset → managed backend (background workloads never inherit BYOK).
    let default_config = Config::default();
    assert_eq!(
        provider_for_role("subconscious", &default_config),
        "openhuman"
    );
}

#[test]
fn make_openhuman_backend_reports_vision_capability() {
    let config = Config::default();
    let (provider, _) = make_openhuman_backend("chat", &config).expect("factory should succeed");
    let caps = provider.capabilities();
    assert!(caps.native_tool_calling);
    assert!(
        caps.vision,
        "OpenHuman backend must report vision so attachment-driven reasoning turns clear the harness gate"
    );
}

#[test]
fn make_openhuman_backend_forwards_raw_byok_model_verbatim() {
    // Issue #4598: a raw/BYOK model id pinned into `default_model` (e.g. a user
    // selecting "claude-opus-4" for an agent) must reach provider construction
    // verbatim rather than the core silently collapsing it onto reasoning-v1.
    // The managed backend preserves non-empty ids and is authoritative over
    // their validity.
    let mut config = Config::default();
    config.default_model = Some("claude-opus-4".to_string());
    let (_, model) = make_openhuman_backend("chat", &config).expect("factory should succeed");
    assert_eq!(
        model, "claude-opus-4",
        "a raw/BYOK default_model must be forwarded verbatim, not collapsed"
    );

    // Another stale/custom id shape from the wild — still forwarded verbatim.
    config.default_model = Some("deepseek-v4-pro".to_string());
    let (_, model) = make_openhuman_backend("chat", &config).expect("factory should succeed");
    assert_eq!(model, "deepseek-v4-pro");
}

#[test]
fn make_openhuman_backend_keeps_valid_tier() {
    let mut config = Config::default();
    config.default_model = Some("chat-v1".to_string());
    let (_, model) = make_openhuman_backend("chat", &config).expect("factory should succeed");
    assert_eq!(model, "chat-v1");
}

#[test]
fn make_openhuman_backend_keeps_reasoning_quick() {
    let mut config = Config::default();
    config.default_model = Some("reasoning-quick-v1".to_string());
    let (_, model) = make_openhuman_backend("chat", &config).expect("factory should succeed");
    assert_eq!(model, "reasoning-quick-v1");
}

#[test]
fn make_openhuman_backend_pins_vision_role_to_vision_tier() {
    // Regression (PR #3699): the managed default_model is chat-v1 (a NON-vision
    // tier). When `vision_provider` is unset the vision workload resolves to the
    // managed backend, so make_openhuman_backend must override the default model
    // with `vision-v1` — otherwise `oh_tier_supports_vision` reports false and
    // the turn engine strips every attached image, blinding the vision sub-agent.
    let config = Config::default();
    assert_eq!(config.default_model.as_deref(), Some("chat-v1"));
    let (_, model) = make_openhuman_backend("vision", &config).expect("factory should succeed");
    assert_eq!(model, crate::openhuman::config::MODEL_VISION_V1);
    assert!(
        oh_tier_supports_vision(&model),
        "vision role must resolve to a vision-capable managed tier"
    );
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
fn burst_role_uses_explicit_agentic_provider() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.agentic_provider = Some("anthropic:claude-haiku-4-5".to_string());

    assert_eq!(
        provider_for_role("burst", &config),
        "anthropic:claude-haiku-4-5",
        "burst workers must preserve explicit agentic provider routing"
    );
}

#[test]
fn burst_role_does_not_inherit_chat_byok_when_agentic_unset() {
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_oai", "openai"));
    config.chat_provider = Some("openai:gpt-4o".to_string());

    assert_eq!(
        provider_for_role("burst", &config),
        "openhuman",
        "unset burst must stay on managed backend rather than inherit chat BYOK"
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

// ── claude_agent_sdk provider factory tests ───────────────────────────────────

#[test]
fn claude_agent_sdk_bare_provider_string_uses_default_model() {
    let config = Config::default();
    let (_, model) = create_chat_provider_from_string("reasoning", "claude_agent_sdk", &config)
        .expect("claude_agent_sdk must build without a model suffix");
    // Default model from ClaudeAgentSdkConfig
    assert_eq!(
        model, "claude-sonnet-4-6",
        "claude_agent_sdk with no suffix must use the default model"
    );
}

#[test]
fn claude_agent_sdk_with_model_suffix() {
    let config = Config::default();
    let (_, model) =
        create_chat_provider_from_string("reasoning", "claude_agent_sdk:claude-opus-4-7", &config)
            .expect("claude_agent_sdk:<model> must build");
    assert_eq!(model, "claude-opus-4-7");
}

#[test]
fn claude_agent_sdk_with_custom_default_model_in_config() {
    let mut config = Config::default();
    config.claude_agent_sdk.default_model = "claude-haiku-4-5".to_string();
    let (_, model) = create_chat_provider_from_string("chat", "claude_agent_sdk", &config)
        .expect("claude_agent_sdk must build with config default model");
    assert_eq!(model, "claude-haiku-4-5");
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

/// Regression guard for TAURI-RUST-59Y: when Ollama returns 404 on
/// `/chat/completions` (e.g. model not found), the provider must NOT
/// attempt a fallback request to `/responses`. The Ollama API has no
/// Responses endpoint, so the fallback produces a second guaranteed-404
/// that previously generated Sentry noise at scale (1,598 events).
///
/// This test mounts a mock server that returns 404 for chat/completions
/// and an empty 200 for the responses endpoint (so we can detect if it
/// was called). After the provider call fails, we assert the responses
/// endpoint received zero requests.
#[tokio::test]
async fn ollama_provider_does_not_fall_back_to_responses_on_404() {
    let mock_server = MockServer::start().await;

    // chat/completions always returns 404 (model not found).
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_string(
            r#"{"error":{"message":"model 'gemma3:1b-it-qat' not found","code":404}}"#,
        ))
        .expect(1) // exactly one attempt — no retry
        .mount(&mock_server)
        .await;

    // /v1/responses should NOT be called — mount with expect(0).
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"output_text":"should not reach here"}"#),
        )
        .expect(0) // must not be called
        .mount(&mock_server)
        .await;

    let mut config = Config::default();
    // Point the Ollama base URL at the mock server.
    config.local_ai.base_url = Some(mock_server.uri());
    let (provider, model) =
        create_chat_provider_from_string("chat", "ollama:gemma3:1b-it-qat", &config)
            .expect("ollama provider must build");

    // The call should fail (404), but must not trigger the /v1/responses path.
    let result = provider.chat_with_system(None, "hello", &model, 0.0).await;
    assert!(
        result.is_err(),
        "provider should fail with 404, got success"
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("404") || err_msg.contains("not found"),
        "error should reference 404/not-found, got: {err_msg}"
    );

    // wiremock verifies expect(0) on the responses mock when the server is dropped.
}

/// Same regression guard as above but for LM Studio — it also lacks the
/// Responses API and must not trigger the fallback on 404.
#[tokio::test]
async fn lmstudio_provider_does_not_fall_back_to_responses_on_404() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(404).set_body_string(r#"{"error":"model not found"}"#))
        .expect(1)
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"output_text":"should not reach here"}"#),
        )
        .expect(0)
        .mount(&mock_server)
        .await;

    let mut config = Config::default();
    config.local_ai.base_url = Some(mock_server.uri());
    let (provider, model) =
        create_chat_provider_from_string("chat", "lmstudio:google/gemma-4-e4b", &config)
            .expect("lmstudio provider must build");

    let result = provider.chat_with_system(None, "hello", &model, 0.0).await;
    assert!(
        result.is_err(),
        "provider should fail with 404, got success"
    );
}

/// Counterpart to the no-fallback tests: a cloud provider (responses_fallback=true)
/// MUST retry against `/v1/responses` when chat/completions returns 404.
/// This guards against an accidental inversion of the supports_responses_fallback flag.
#[tokio::test]
async fn cloud_provider_falls_back_to_responses_on_404() {
    let mock_server = MockServer::start().await;

    // chat/completions returns 404 → should trigger fallback.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"error":{"message":"model not found","code":404}}"#),
        )
        .expect(1) // exactly one attempt
        .mount(&mock_server)
        .await;

    // /v1/responses MUST be called — the provider should fall back to it.
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"output":[{"content":[{"type":"output_text","text":"ok"}]}]}"#,
            ),
        )
        .expect(1) // must be called exactly once
        .mount(&mock_server)
        .await;

    // Use AuthStyle::None so no API key lookup is needed.
    // The endpoint must include /v1 so that chat_completions_url() resolves to
    // /v1/chat/completions and responses_url() resolves to /v1/responses.
    let config = config_with_providers(vec![CloudProviderCreds {
        id: "p_test".to_string(),
        slug: "test-cloud".to_string(),
        label: "Test Cloud".to_string(),
        endpoint: format!("{}/v1", mock_server.uri()),
        auth_style: AuthStyle::None,
        default_model: Some("test-model".to_string()),
        ..Default::default()
    }]);

    let (provider, model) =
        create_chat_provider_from_string("chat", "test-cloud:test-model", &config)
            .expect("cloud provider must build");

    // The call should succeed via the responses fallback.
    let result = provider.chat_with_system(None, "hello", &model, 0.0).await;

    // wiremock verifies expect(1) on the responses mock when the server is dropped.
    // We don't assert Ok here because the provider may return an error even after a
    // successful fallback call (e.g. if the response body doesn't fully satisfy parsing).
    // The important invariant is that /v1/responses was called — verified by wiremock.
    drop(result);
}

/// TAURI-RUST-5EN: a built-in chat-completions-only cloud provider (DeepSeek)
/// must NOT fall back to `/v1/responses` on a chat-completions 404. DeepSeek
/// exposes no Responses API, so the fallback is a guaranteed second 404 that
/// floods Sentry with an empty-body "deepseek Responses API error:" event.
/// Bearer-path counterpart to `ollama_provider_does_not_fall_back_to_responses_on_404`.
#[tokio::test]
async fn deepseek_builtin_does_not_fall_back_to_responses_on_404() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"error":{"message":"model not found","code":404}}"#),
        )
        .expect(1) // exactly one attempt — no retry, no fallback
        .mount(&mock_server)
        .await;

    // DeepSeek has no /v1/responses — the fallback must never reach it.
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(404).set_body_string(""))
        .expect(0) // must not be called
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().expect("tempdir");
    let entry = CloudProviderCreds {
        id: "p_deepseek".to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: format!("{}/v1", mock_server.uri()),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-v4-flash".to_string()),
        ..Default::default()
    };
    let config = config_with_providers_in_tempdir(&tmp, vec![entry]);
    // Bearer providers fail at call time with "API key not set" before any HTTP
    // request, so stash a key to let the chat-completions call reach the mock.
    AuthService::from_config(&config)
        .store_provider_token(
            "provider:deepseek",
            "default",
            "sk-test",
            Default::default(),
            true,
        )
        .expect("store provider token");

    let (provider, model) =
        create_chat_provider_from_string("chat", "deepseek:deepseek-v4-flash", &config)
            .expect("deepseek provider must build");

    let result = provider.chat_with_system(None, "hello", &model, 0.0).await;
    assert!(
        result.is_err(),
        "chat-completions 404 should surface as an error, not a success"
    );

    // wiremock verifies expect(0) on /v1/responses when the server is dropped.
}

/// Counterpart guard: a custom (non-built-in) Bearer slug KEEPS the responses
/// fallback — its endpoint may be a genuine OpenAI proxy that serves
/// `/v1/responses`. Ensures the 5EN slug-gate only disables the fallback for
/// known chat-completions-only built-ins, not for unknown providers.
#[tokio::test]
async fn custom_bearer_provider_keeps_responses_fallback_on_404() {
    let mock_server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(404)
                .set_body_string(r#"{"error":{"message":"model not found","code":404}}"#),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    // Unknown slug → fallback retained → /v1/responses MUST be called.
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_string(
                r#"{"output":[{"content":[{"type":"output_text","text":"ok"}]}]}"#,
            ),
        )
        .expect(1)
        .mount(&mock_server)
        .await;

    let tmp = TempDir::new().expect("tempdir");
    let entry = CloudProviderCreds {
        id: "p_custom".to_string(),
        slug: "my-openai-proxy".to_string(),
        label: "My Proxy".to_string(),
        endpoint: format!("{}/v1", mock_server.uri()),
        auth_style: AuthStyle::Bearer,
        default_model: Some("proxy-model".to_string()),
        ..Default::default()
    };
    let config = config_with_providers_in_tempdir(&tmp, vec![entry]);
    AuthService::from_config(&config)
        .store_provider_token(
            "provider:my-openai-proxy",
            "default",
            "sk-test",
            Default::default(),
            true,
        )
        .expect("store provider token");

    let (provider, model) =
        create_chat_provider_from_string("chat", "my-openai-proxy:proxy-model", &config)
            .expect("custom bearer provider must build");

    let result = provider.chat_with_system(None, "hello", &model, 0.0).await;
    drop(result);

    // wiremock verifies expect(1) on /v1/responses when the server is dropped.
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
                max_tokens: None,
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
                max_tokens: None,
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

// ── nvidia-nim / empty-model guard tests (issue #2784) ─────────────────────

/// Helper: build a minimal nvidia-nim-style cloud provider entry.
fn nvidia_nim_entry(id: &str, default_model: Option<&str>) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "nvidia-nim".to_string(),
        label: "NVIDIA NIM".to_string(),
        endpoint: "https://integrate.api.nvidia.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: default_model.map(ToString::to_string),
        ..Default::default()
    }
}

/// When the provider string includes a model id the factory should build
/// successfully and return that model id unchanged.
#[test]
fn nvidia_nim_with_explicit_model_builds_correctly() {
    let config = config_with_providers(vec![nvidia_nim_entry("p_nim", None)]);
    let (_, model) = create_chat_provider_from_string(
        "reasoning",
        "nvidia-nim:meta/llama-3.1-8b-instruct",
        &config,
    )
    .expect("nvidia-nim with explicit model must build");
    assert_eq!(
        model, "meta/llama-3.1-8b-instruct",
        "model id must pass through unchanged"
    );
}

/// When the provider string has no model id (`"nvidia-nim:"`) and no
/// default_model is configured, the factory must fail with a clear error
/// rather than silently sending an empty model string to the API (which
/// triggers a 400 "model field is required" from nvidia-nim).
///
/// Regression test for https://github.com/tinyhumansai/openhuman/issues/2784.
#[test]
fn nvidia_nim_empty_model_in_provider_string_errors_clearly() {
    let config = config_with_providers(vec![nvidia_nim_entry("p_nim", None)]);
    let err = match create_chat_provider_from_string("reasoning", "nvidia-nim:", &config) {
        Ok(_) => panic!("empty model string must not succeed — would send model='' to the API"),
        Err(e) => e,
    };
    let msg = err.to_string();
    assert!(
        msg.contains("empty model id"),
        "error must mention empty model id, got: {msg}"
    );
    assert!(
        msg.contains("nvidia-nim"),
        "error must name the provider slug, got: {msg}"
    );
}

/// When the provider string has no model id but the entry has a concrete
/// default_model, that default should be used — no error.
#[test]
fn nvidia_nim_falls_back_to_default_model_when_no_model_in_string() {
    let config = config_with_providers(vec![nvidia_nim_entry(
        "p_nim",
        Some("meta/llama-3.1-70b-instruct"),
    )]);
    let (_, model) = create_chat_provider_from_string("reasoning", "nvidia-nim:", &config)
        .expect("nvidia-nim: with default_model configured must build");
    assert_eq!(
        model, "meta/llama-3.1-70b-instruct",
        "should fall back to default_model from config entry"
    );
}

// ── config.api_key fallback scoping (PR #2724) ───────────────────────────

/// Build a tempdir-backed Config with a global `config.api_key`, a custom
/// `inference_url`, and two cloud providers: one whose endpoint matches the
/// inference_url (the legacy direct-inference slug) and one that does not.
///
/// The tempdir workspace has no stored auth-profiles, so `lookup_key_for_slug`
/// exhausts the standard auth path and reaches the `config.api_key` fallback.
fn config_for_api_key_fallback(tmp: &TempDir) -> Config {
    let mut custom = openai_entry("p_custom", "custom");
    custom.endpoint = "https://inference.example.com/v1".to_string();
    let config = config_with_providers_in_tempdir(
        tmp,
        vec![custom, anthropic_entry("p_anthropic", "anthropic")],
    );
    let mut config = config;
    config.api_key = Some("global-key".to_string());
    config.inference_url = Some("https://inference.example.com/v1".to_string());
    config
}

/// The legacy direct-inference slug — the provider whose endpoint matches
/// `config.inference_url` — inherits the global `config.api_key`.
#[test]
fn config_api_key_fallback_applies_to_legacy_inference_slug() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_for_api_key_fallback(&tmp);
    assert_eq!(
        lookup_key_for_slug("custom", &config).expect("lookup must succeed"),
        "global-key",
        "legacy direct-inference slug must inherit config.api_key fallback",
    );
}

/// Load-bearing negative assertion: a provider whose endpoint does NOT match
/// `config.inference_url` must NOT inherit the global `config.api_key`.
/// Without this guard the fallback would leak one provider's credential to
/// every other provider (cross-provider credential leak, PR #2724).
#[test]
fn config_api_key_fallback_does_not_leak_to_other_slugs() {
    let tmp = TempDir::new().expect("tempdir");
    let config = config_for_api_key_fallback(&tmp);
    assert_eq!(
        lookup_key_for_slug("anthropic", &config).expect("lookup must succeed"),
        "",
        "non-matching slug must NOT inherit config.api_key — would leak credentials",
    );
}

/// When `inference_url` itself is unset, the `config.api_key` fallback never
/// fires (no legacy direct-inference slug to scope to), so no slug inherits it.
#[test]
fn config_api_key_fallback_inert_without_inference_url() {
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_for_api_key_fallback(&tmp);
    config.inference_url = None;
    assert_eq!(
        lookup_key_for_slug("custom", &config).expect("lookup must succeed"),
        "",
        "without inference_url there is no legacy slug — fallback must stay inert",
    );
}

// ── Local provider profile tests ─────────────────────────────────────────────

#[test]
fn mlx_provider_string_resolves() {
    let config = Config::default();
    let result = create_chat_provider_from_string("chat", "mlx:llama-3.1-8b", &config);
    assert!(result.is_ok(), "mlx provider must resolve");
    let (_, model) = result.unwrap();
    assert_eq!(model, "llama-3.1-8b");
}

#[test]
fn local_openai_provider_string_resolves() {
    let config = Config::default();
    let result = create_chat_provider_from_string("chat", "local-openai:phi3", &config);
    assert!(result.is_ok(), "local-openai provider must resolve");
    let (_, model) = result.unwrap();
    assert_eq!(model, "phi3");
}

#[test]
fn mlx_provider_empty_model_errors() {
    let config = Config::default();
    let result = create_chat_provider_from_string("chat", "mlx:", &config);
    let err = result.err().expect("mlx: with empty model must error");
    assert!(err.to_string().contains("empty model"));
}

#[test]
fn local_openai_provider_empty_model_errors() {
    let config = Config::default();
    let result = create_chat_provider_from_string("chat", "local-openai:", &config);
    let err = result
        .err()
        .expect("local-openai: with empty model must error");
    assert!(err.to_string().contains("empty model"));
}

#[test]
fn ollama_provider_passes_num_ctx() {
    let mut config = Config::default();
    config.local_ai.num_ctx = Some(32768);
    let result = create_chat_provider_from_string("chat", "ollama:qwen3:14b", &config);
    assert!(result.is_ok());
    // The provider is constructed — num_ctx is set on the provider instance.
    // Full integration test verifying the serialized body is in the JSON-RPC
    // E2E suite; here we just confirm the factory doesn't reject it.
}

#[test]
fn byok_fallback_skips_mlx_and_local_openai() {
    let mut config = Config::default();
    config.chat_provider = Some("mlx:llama3".to_string());
    config.reasoning_provider = Some("local-openai:phi3".to_string());
    // Neither should be picked up as a BYOK fallback
    let result = resolve_byok_fallback_provider_string(&config);
    assert!(
        result.is_none(),
        "local providers must not be BYOK fallbacks"
    );
}

#[test]
fn byok_fallback_skips_omlx() {
    let mut config = Config::default();
    config.chat_provider = Some("omlx:llama3".to_string());

    assert!(
        resolve_byok_fallback_provider_string(&config).is_none(),
        "OMLX is a local provider and must not be treated as a BYOK cloud fallback"
    );
    assert_eq!(
        provider_for_role("coding", &config),
        "openhuman",
        "unset coding must not inherit chat OMLX as a BYOK fallback"
    );
}

#[test]
fn local_provider_string_detection() {
    use crate::openhuman::inference::local::profile::is_local_provider_string;
    assert!(is_local_provider_string("ollama:phi3"));
    assert!(is_local_provider_string("lmstudio:model"));
    assert!(is_local_provider_string("mlx:llama"));
    assert!(is_local_provider_string("omlx:llama"));
    assert!(is_local_provider_string("local-openai:qwen2"));
    assert!(!is_local_provider_string("openai:gpt-4o"));
    assert!(!is_local_provider_string("openhuman"));
    assert!(!is_local_provider_string("cloud"));
}

// ── resolve_model_for_hint ──────────────────────────────────────────────

#[test]
fn resolve_model_for_hint_maps_known_hints_to_tiers() {
    let config = Config::default();
    assert_eq!(
        resolve_model_for_hint("hint:reasoning", &config),
        "reasoning-v1"
    );
    assert_eq!(resolve_model_for_hint("hint:chat", &config), "chat-v1");
    assert_eq!(
        resolve_model_for_hint("hint:agentic", &config),
        "agentic-v1"
    );
    assert_eq!(resolve_model_for_hint("hint:burst", &config), "burst-v1");
    assert_eq!(resolve_model_for_hint("hint:coding", &config), "coding-v1");
    assert_eq!(
        resolve_model_for_hint("hint:summarization", &config),
        "summarization-v1"
    );
}

#[test]
fn resolve_model_for_hint_passes_through_tier_names() {
    let config = Config::default();
    assert_eq!(
        resolve_model_for_hint("reasoning-v1", &config),
        "reasoning-v1"
    );
    assert_eq!(resolve_model_for_hint("agentic-v1", &config), "agentic-v1");
    assert_eq!(resolve_model_for_hint("coding-v1", &config), "coding-v1");
}

#[test]
fn resolve_model_for_hint_extracts_model_from_byok_provider() {
    let mut config = Config::default();
    config.reasoning_provider = Some("openai:gpt-4o".to_string());
    assert_eq!(resolve_model_for_hint("hint:reasoning", &config), "gpt-4o");

    config.chat_provider = Some("anthropic:claude-sonnet-4-20250514".to_string());
    assert_eq!(
        resolve_model_for_hint("hint:chat", &config),
        "claude-sonnet-4-20250514"
    );
}

#[test]
fn resolve_model_for_hint_falls_through_openhuman_and_cloud_sentinels() {
    let mut config = Config::default();
    config.reasoning_provider = Some("openhuman".to_string());
    assert_eq!(
        resolve_model_for_hint("hint:reasoning", &config),
        "reasoning-v1"
    );

    config.reasoning_provider = Some("cloud".to_string());
    assert_eq!(
        resolve_model_for_hint("hint:reasoning", &config),
        "reasoning-v1"
    );

    config.reasoning_provider = Some("".to_string());
    assert_eq!(
        resolve_model_for_hint("hint:reasoning", &config),
        "reasoning-v1"
    );
}

#[test]
fn resolve_model_for_hint_handles_unknown_hint_passthrough() {
    let config = Config::default();
    let result = resolve_model_for_hint("hint:unknown_tier", &config);
    assert_eq!(result, "hint:unknown_tier");
}

#[test]
fn resolve_model_for_hint_subconscious_managed_is_chat_v1() {
    // Managed (no BYOK subconscious_provider) resolves to the chat tier model so
    // the RPC `inference.resolve_model` reports the model the tick actually runs.
    let config = Config::default();
    assert_eq!(
        resolve_model_for_hint("hint:subconscious", &config),
        "chat-v1"
    );

    // An explicit managed sentinel still resolves to the tier, not the raw hint.
    let mut config = Config::default();
    config.subconscious_provider = Some("openhuman".to_string());
    assert_eq!(
        resolve_model_for_hint("hint:subconscious", &config),
        "chat-v1"
    );
}

#[test]
fn resolve_model_for_hint_subconscious_reads_subconscious_provider() {
    // The `subconscious` hint must read `subconscious_provider` — NOT the
    // chat-tier provider it shares a model with — so a BYOK subconscious route
    // surfaces its own model id.
    let mut config = Config::default();
    config.subconscious_provider = Some("openai:gpt-4o-mini".to_string());
    // A different chat_provider must not leak into the subconscious resolution.
    config.chat_provider = Some("anthropic:claude-sonnet-4-20250514".to_string());
    assert_eq!(
        resolve_model_for_hint("hint:subconscious", &config),
        "gpt-4o-mini"
    );
}

// ── role_for_model_tier ─────────────────────────────────────────────────

#[test]
fn role_for_model_tier_maps_tier_names_to_roles() {
    // The demo flow pins these two tiers on its agent nodes; they must route to
    // the reasoning and chat workloads respectively.
    assert_eq!(role_for_model_tier("reasoning-v1"), "reasoning");
    assert_eq!(role_for_model_tier("chat-v1"), "chat");
    assert_eq!(role_for_model_tier("agentic-v1"), "agentic");
    assert_eq!(role_for_model_tier("burst-v1"), "burst");
    assert_eq!(role_for_model_tier("coding-v1"), "coding");
    assert_eq!(role_for_model_tier("vision-v1"), "vision");
    assert_eq!(role_for_model_tier("summarization-v1"), "summarization");
    // The quick reasoning tier shares the chat workload for its model.
    assert_eq!(role_for_model_tier("reasoning-quick-v1"), "chat");
}

#[test]
fn role_for_model_tier_normalises_hint_aliases() {
    assert_eq!(role_for_model_tier("hint:reasoning"), "reasoning");
    assert_eq!(role_for_model_tier("hint:chat"), "chat");
    assert_eq!(role_for_model_tier("hint:coding"), "coding");
    // Subconscious rides the chat tier's model.
    assert_eq!(role_for_model_tier("hint:subconscious"), "chat");
}

#[test]
fn role_for_model_tier_unknown_falls_back_to_chat() {
    assert_eq!(role_for_model_tier("gpt-4o"), "chat");
    assert_eq!(role_for_model_tier("hint:unknown_tier"), "chat");
    assert_eq!(role_for_model_tier(""), "chat");
}

#[test]
fn omlx_provider_builds_with_bearer_key() {
    let mut config = crate::openhuman::config::Config::default();
    config.local_ai.api_key = Some("sk-omlx-test".to_string());
    config.local_ai.base_url = Some("http://127.0.0.1:8000/v1".to_string());
    let (_provider, model) =
        super::make_omlx_provider("my-model", None, &config).expect("omlx provider builds");
    assert_eq!(model, "my-model");
}

#[test]
fn omlx_dispatch_empty_model_errors() {
    // Covers the empty-model bail! arms in create_chat_provider_from_string
    // and create_local_chat_provider_from_string for the "omlx:" prefix.
    let config = crate::openhuman::config::Config::default();

    let err = create_chat_provider_from_string("chat", "omlx:", &config)
        .err()
        .expect("omlx: with empty model must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("empty model") || msg.contains("omlx:<model"),
        "expected empty-model diagnostic, got: {msg}"
    );

    let err_local = create_local_chat_provider_from_string("omlx:", &config)
        .err()
        .expect("omlx: with empty model must fail via local dispatch");
    let msg_local = err_local.to_string();
    assert!(
        msg_local.contains("empty model") || msg_local.contains("omlx:<model"),
        "expected empty-model diagnostic from local dispatch, got: {msg_local}"
    );
}

#[test]
fn omlx_provider_builds_without_key_uses_no_auth() {
    // Covers the no-api_key warn branch in make_omlx_provider — must not panic,
    // must return Ok with the correct model name.
    let mut config = crate::openhuman::config::Config::default();
    config.local_ai.api_key = None;
    config.local_ai.base_url = Some("http://127.0.0.1:8000/v1".to_string());
    let (_provider, model) =
        super::make_omlx_provider("m", None, &config).expect("omlx provider builds without key");
    assert_eq!(model, "m");
}

#[test]
fn omlx_dispatch_success_builds_provider() {
    // Covers the success arms (non-empty model -> make_omlx_provider) in both
    // create_chat_provider_from_string and create_local_chat_provider_from_string.
    let mut config = crate::openhuman::config::Config::default();
    config.local_ai.api_key = Some("sk-omlx-test".to_string());
    config.local_ai.base_url = Some("http://127.0.0.1:8000/v1".to_string());

    let (_p, model) = create_chat_provider_from_string("chat", "omlx:my-model", &config)
        .expect("omlx:<model> builds via public factory");
    assert_eq!(model, "my-model");

    let (_p_local, model_local) = create_local_chat_provider_from_string("omlx:my-model", &config)
        .expect("omlx:<model> builds via local dispatch");
    assert_eq!(model_local, "my-model");
}

// ── #3767: managed-credits gate bypass (gate-only, per-tier) ───────────────
//
// Routing is NOT changed by this fix — selecting a BYO provider already routes
// inference correctly. The gate is evaluated PER TIER so the UI checks whichever
// tier the user actually selected: the chat header's "Quick" mode runs on the
// `chat` tier and "Reasoning" mode on the `reasoning` tier. `role_bypasses_
// managed_credits(role)` is true when that role runs on the user's own funding
// (a BYO cloud key, a local runtime, or claude-code) with usable credentials.
// Tiers that stay managed and run anyway surface the per-call 402 error.

/// Store a usable provider key under the new-style `provider:<slug>` profile so
/// `lookup_key_for_slug` resolves it.
fn store_byo_key(config: &Config, slug: &str, token: &str) {
    let auth = AuthService::from_config(config);
    auth.store_provider_token(
        &format!("provider:{slug}"),
        "default",
        token,
        Default::default(),
        true,
    )
    .expect("store provider token");
}

#[test]
fn byo_chat_tier_with_key_bypasses() {
    let tmp = TempDir::new().expect("tempdir");
    // Quick mode runs on `chat`; routed to the user's own OpenAI provider + key.
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("openai:gpt-4o".to_string());
    store_byo_key(&config, "openai", "sk-byo-test");

    assert!(role_bypasses_managed_credits("chat", &config));
}

#[test]
fn byo_reasoning_tier_with_key_bypasses() {
    let tmp = TempDir::new().expect("tempdir");
    // Reasoning mode runs on `reasoning`; routed to the user's own provider + key.
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.reasoning_provider = Some("openai:gpt-4o".to_string());
    store_byo_key(&config, "openai", "sk-byo-test");

    assert!(role_bypasses_managed_credits("reasoning", &config));
}

#[test]
fn per_tier_diverges_chat_byo_reasoning_managed() {
    let tmp = TempDir::new().expect("tempdir");
    // The crux of the per-tier check: chat on BYOK, reasoning explicitly managed.
    // Quick mode (chat) bypasses; Reasoning mode (reasoning) stays gated.
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("openai:gpt-4o".to_string());
    config.reasoning_provider = Some("openhuman".to_string());
    store_byo_key(&config, "openai", "sk-byo-test");

    assert!(role_bypasses_managed_credits("chat", &config));
    assert!(!role_bypasses_managed_credits("reasoning", &config));
}

#[test]
fn local_tier_bypasses_without_any_key() {
    // A tier on a local on-device runtime → bypass, no cloud key needed.
    let mut config = Config::default();
    config.chat_provider = Some("ollama:qwen3:8b".to_string());
    assert!(role_bypasses_managed_credits("chat", &config));
}

#[test]
fn managed_chat_with_byo_agentic_stays_gated() {
    let tmp = TempDir::new().expect("tempdir");
    // chat explicitly managed; only tool-use (agentic) is BYOK. The chat tier
    // still bills managed credits → chat role stays gated. (agentic itself is a
    // BYO route, but it is not a chat-mode tier and surfaces errors per-call.)
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("openhuman".to_string());
    config.reasoning_provider = Some("openhuman".to_string());
    config.agentic_provider = Some("openai:gpt-4o".to_string());
    store_byo_key(&config, "openai", "sk-byo-test");

    assert!(!role_bypasses_managed_credits("chat", &config));
    assert!(!role_bypasses_managed_credits("reasoning", &config));
}

#[test]
fn managed_chat_with_byo_vision_stays_gated() {
    let tmp = TempDir::new().expect("tempdir");
    // Vision on BYOK but the chat-mode tiers stay managed → chat/reasoning gated.
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("openhuman".to_string());
    config.reasoning_provider = Some("openhuman".to_string());
    config.vision_provider = Some("openai:gpt-4o".to_string());
    store_byo_key(&config, "openai", "sk-byo-test");

    assert!(!role_bypasses_managed_credits("chat", &config));
    assert!(!role_bypasses_managed_credits("reasoning", &config));
}

#[test]
fn no_byo_provider_stays_gated() {
    let tmp = TempDir::new().expect("tempdir");
    // OpenAI entry exists but every tier is left on the managed default and no
    // key is stored → chat-mode tiers managed → must NOT bypass.
    let config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);

    assert_eq!(provider_for_role("chat", &config), "openhuman");
    assert!(!role_bypasses_managed_credits("chat", &config));
    assert!(!role_bypasses_managed_credits("reasoning", &config));
}

#[test]
fn default_config_with_no_key_stays_gated() {
    // No BYO provider at all → both chat-mode tiers gated.
    let config = Config::default();
    assert!(!role_bypasses_managed_credits("chat", &config));
    assert!(!role_bypasses_managed_credits("reasoning", &config));
}

#[test]
fn byo_route_without_usable_key_stays_gated() {
    let tmp = TempDir::new().expect("tempdir");
    // chat tier points at a BYO slug with NO stored key — the route would fail
    // with an auth error, not bill managed credits, but we must not bypass for a
    // route that cannot run on the user's dime (#3767: "BYO key present but
    // invalid/unverified → still gated").
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("openai:gpt-4o".to_string());

    // The explicit route is still honored verbatim by provider_for_role…
    assert_eq!(provider_for_role("chat", &config), "openai:gpt-4o");
    // …but with no usable key the gate stays on.
    assert!(!role_bypasses_managed_credits("chat", &config));

    // Once a key is stored, the route becomes a genuine bypass.
    store_byo_key(&config, "openai", "sk-byo-test");
    assert!(role_bypasses_managed_credits("chat", &config));
}

// ── Privacy Mode: local-only inference enforcement (#4435) ───────────────────

#[test]
fn local_only_blocks_external_cloud_slug() {
    use crate::openhuman::config::PrivacyMode;
    let v = local_only_violation(PrivacyMode::LocalOnly, "openai:gpt-4o");
    assert_eq!(v.as_deref(), Some("openai"));
}

#[test]
fn local_only_blocks_managed_backend() {
    use crate::openhuman::config::PrivacyMode;
    let v = local_only_violation(PrivacyMode::LocalOnly, PROVIDER_OPENHUMAN);
    assert_eq!(v.as_deref(), Some("OpenHuman (managed cloud)"));
}

#[test]
fn local_only_blocks_claude_code_cli() {
    use crate::openhuman::config::PrivacyMode;
    let v = local_only_violation(PrivacyMode::LocalOnly, "claude-code:sonnet");
    assert_eq!(v.as_deref(), Some("Claude Code CLI"));
}

#[test]
fn local_only_permits_local_runtimes() {
    use crate::openhuman::config::PrivacyMode;
    for local in [
        "ollama:llama3",
        "lmstudio:qwen",
        "mlx:phi",
        "local-openai:foo",
    ] {
        assert_eq!(
            local_only_violation(PrivacyMode::LocalOnly, local),
            None,
            "local provider '{local}' must be permitted in LocalOnly mode"
        );
    }
}

#[test]
fn local_only_defers_reresolving_sentinels() {
    use crate::openhuman::config::PrivacyMode;
    // Empty / "cloud" re-resolve to a concrete string and are re-checked on the
    // recursive call — not blocked here.
    assert_eq!(local_only_violation(PrivacyMode::LocalOnly, ""), None);
    assert_eq!(local_only_violation(PrivacyMode::LocalOnly, "cloud"), None);
}

#[test]
fn standard_mode_permits_external() {
    use crate::openhuman::config::PrivacyMode;
    assert_eq!(
        local_only_violation(PrivacyMode::Standard, "openai:gpt-4o"),
        None
    );
    assert_eq!(
        local_only_violation(PrivacyMode::Sensitive, "openai:gpt-4o"),
        None,
        "Sensitive mode has no egress enforcement in S1"
    );
}

#[test]
fn enforce_local_only_inference_errors_on_external_when_local_only() {
    // Drive the live-policy-backed wrapper: install a LocalOnly policy, then
    // assert an external provider is refused with the privacy message and a
    // local provider passes.
    let _env = crate::openhuman::config::TEST_ENV_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    use crate::openhuman::config::PrivacyMode;
    use crate::openhuman::security::SecurityPolicy;
    let ws = std::env::temp_dir().join("openhuman_factory_privacy_test");
    let policy = std::sync::Arc::new(
        SecurityPolicy {
            workspace_dir: ws.clone(),
            ..SecurityPolicy::default()
        }
        .with_privacy_mode(PrivacyMode::LocalOnly),
    );
    crate::openhuman::security::live_policy::install(policy, ws.clone(), ws.clone());

    let err = enforce_local_only_inference("chat", "openai:gpt-4o")
        .expect_err("external provider must be refused in LocalOnly mode");
    let msg = err.to_string();
    assert!(
        msg.contains("Local-only privacy mode is active"),
        "unexpected error: {msg}"
    );
    assert!(
        msg.contains("openai"),
        "error should name the provider: {msg}"
    );

    // Local provider passes.
    enforce_local_only_inference("chat", "ollama:llama3")
        .expect("local provider must be permitted in LocalOnly mode");

    // Restore Standard so we don't leak LocalOnly into other serial tests.
    crate::openhuman::security::live_policy::reload_privacy(PrivacyMode::Standard)
        .expect("policy installed");
}

// ── Phase 1 (#4249): `create_chat_model` seam ──────────────────────────────
// The crate `ChatModel` factory wraps the resolved `Provider` via
// `ProviderModel` with zero behaviour change; a one-shot `invoke` must
// round-trip through the underlying provider.
#[tokio::test]
async fn create_chat_model_wraps_provider_and_round_trips() {
    use crate::openhuman::inference::provider::traits::Provider;
    use async_trait::async_trait;
    use std::sync::Arc;
    use tinyagents::harness::message::Message;
    use tinyagents::harness::model::{ChatModel, ModelRequest};

    let _guard = crate::openhuman::inference::inference_test_guard();

    struct CannedProvider;
    #[async_trait]
    impl Provider for CannedProvider {
        async fn chat_with_system(
            &self,
            _system_prompt: Option<&str>,
            message: &str,
            _model: &str,
            _temperature: f64,
        ) -> anyhow::Result<String> {
            Ok(format!("echo: {message}"))
        }
    }

    // The factory consults this override under cfg(test), so `create_chat_model`
    // resolves to the mock without needing configured cloud providers.
    let _override = test_provider_override::install(Arc::new(CannedProvider));
    let config = Config::default();

    let model = create_chat_model("chat", &config, 0.3).expect("create_chat_model must build");
    let response = model
        .invoke(&(), ModelRequest::new(vec![Message::user("hi there")]))
        .await
        .expect("invoke must succeed");
    assert_eq!(response.text(), "echo: hi there");
}
