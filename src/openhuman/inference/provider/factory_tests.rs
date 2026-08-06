use super::*;
use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
use crate::openhuman::config::Config;
use crate::openhuman::security::credentials::AuthService;
use tempfile::TempDir;

fn create_test_chat_model_from_string(
    role: &str,
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    create_chat_model_from_string_with_model_id(role, provider, config, 0.7)
}

fn create_test_local_chat_model_from_string(
    provider: &str,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    create_local_chat_model_from_string(provider, config)
}

fn create_test_omlx_model(
    model: &str,
    _temperature: Option<f64>,
    config: &Config,
) -> anyhow::Result<(Arc<dyn ChatModel<()>>, String)> {
    create_local_chat_model_from_string(&format!("omlx:{model}"), config)
}

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
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = config_with_providers(vec![nvidia_nim_entry("p_nim", None)]);
    let (_, model) = create_test_chat_model_from_string(
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
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = config_with_providers(vec![nvidia_nim_entry("p_nim", None)]);
    let err = match create_test_chat_model_from_string("reasoning", "nvidia-nim:", &config) {
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
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = config_with_providers(vec![nvidia_nim_entry(
        "p_nim",
        Some("meta/llama-3.1-70b-instruct"),
    )]);
    let (_, model) = create_test_chat_model_from_string("reasoning", "nvidia-nim:", &config)
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
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = Config::default();
    let result = create_test_chat_model_from_string("chat", "mlx:llama-3.1-8b", &config);
    assert!(result.is_ok(), "mlx provider must resolve");
    let (_, model) = result.unwrap();
    assert_eq!(model, "llama-3.1-8b");
}

#[test]
fn local_openai_provider_string_resolves() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = Config::default();
    let result = create_test_chat_model_from_string("chat", "local-openai:phi3", &config);
    assert!(result.is_ok(), "local-openai provider must resolve");
    let (_, model) = result.unwrap();
    assert_eq!(model, "phi3");
}

#[test]
fn mlx_provider_empty_model_errors() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = Config::default();
    let result = create_test_chat_model_from_string("chat", "mlx:", &config);
    let err = result.err().expect("mlx: with empty model must error");
    assert!(err.to_string().contains("empty model"));
}

#[test]
fn local_openai_provider_empty_model_errors() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = Config::default();
    let result = create_test_chat_model_from_string("chat", "local-openai:", &config);
    let err = result
        .err()
        .expect("local-openai: with empty model must error");
    assert!(err.to_string().contains("empty model"));
}

#[test]
fn ollama_provider_passes_num_ctx() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.local_ai.num_ctx = Some(32768);
    let result = create_test_chat_model_from_string("chat", "ollama:qwen3:14b", &config);
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
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = crate::openhuman::config::Config::default();
    config.local_ai.api_key = Some("sk-omlx-test".to_string());
    config.local_ai.base_url = Some("http://127.0.0.1:8000/v1".to_string());
    let (_provider, model) =
        create_test_omlx_model("my-model", None, &config).expect("omlx provider builds");
    assert_eq!(model, "my-model");
}

#[test]
fn omlx_dispatch_empty_model_errors() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // Covers the empty-model bail! arms in create_test_chat_model_from_string
    // and create_test_local_chat_model_from_string for the "omlx:" prefix.
    let config = crate::openhuman::config::Config::default();

    let err = create_test_chat_model_from_string("chat", "omlx:", &config)
        .err()
        .expect("omlx: with empty model must fail");
    let msg = err.to_string();
    assert!(
        msg.contains("empty model") || msg.contains("omlx:<model"),
        "expected empty-model diagnostic, got: {msg}"
    );

    let err_local = create_test_local_chat_model_from_string("omlx:", &config)
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
    let _guard = crate::openhuman::inference::inference_test_guard();
    // Covers the no-api_key OMLX builder branch — must not panic and must
    // return Ok with the correct model name.
    let mut config = crate::openhuman::config::Config::default();
    config.local_ai.api_key = None;
    config.local_ai.base_url = Some("http://127.0.0.1:8000/v1".to_string());
    let (_provider, model) =
        create_test_omlx_model("m", None, &config).expect("omlx provider builds without key");
    assert_eq!(model, "m");
}

#[test]
fn omlx_dispatch_success_builds_provider() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // Covers the non-empty OMLX model success arms in both
    // create_test_chat_model_from_string and create_test_local_chat_model_from_string.
    let mut config = crate::openhuman::config::Config::default();
    config.local_ai.api_key = Some("sk-omlx-test".to_string());
    config.local_ai.base_url = Some("http://127.0.0.1:8000/v1".to_string());

    let (_p, model) = create_test_chat_model_from_string("chat", "omlx:my-model", &config)
        .expect("omlx:<model> builds via public factory");
    assert_eq!(model, "my-model");

    let (_p_local, model_local) =
        create_test_local_chat_model_from_string("omlx:my-model", &config)
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
fn local_only_blocks_claude_agent_sdk() {
    use crate::openhuman::config::PrivacyMode;
    let violation = local_only_violation(PrivacyMode::LocalOnly, "claude_agent_sdk:sonnet");
    assert_eq!(violation.as_deref(), Some("Claude Agent SDK"));
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
    // local provider passes. Factory tests use `inference_test_guard`; take the
    // same lock before mutating the process-global live policy so parallel
    // cloud-model construction cannot observe this temporary LocalOnly mode.
    let _inference = crate::openhuman::inference::inference_test_guard();
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

    let sdk_error = match create_chat_model_from_string(
        "chat",
        "claude_agent_sdk:claude-sonnet-4-6",
        &Config::default(),
        0.0,
    ) {
        Err(error) => error,
        Ok(_) => panic!("direct Claude SDK model must preserve the privacy gate"),
    };
    assert!(sdk_error.to_string().contains("Local-only privacy mode"));

    let claude_code_error = match create_chat_model_from_string(
        "coding",
        "claude-code:claude-sonnet-4-6",
        &Config::default(),
        0.0,
    ) {
        Err(error) => error,
        Ok(_) => panic!("direct Claude Code model must preserve the privacy gate"),
    };
    assert!(claude_code_error
        .to_string()
        .contains("Local-only privacy mode"));

    // Local provider passes.
    enforce_local_only_inference("chat", "ollama:llama3")
        .expect("local provider must be permitted in LocalOnly mode");

    // Restore Standard so we don't leak LocalOnly into other serial tests.
    crate::openhuman::security::live_policy::reload_privacy(PrivacyMode::Standard)
        .expect("policy installed");
}

// ── Phase 1 (#4249): `create_chat_model` seam ──────────────────────────────
// The crate `ChatModel` factory must return the injected crate-native model
// directly; a one-shot `invoke` round-trips without a Provider adapter.
#[tokio::test]
async fn create_chat_model_uses_native_test_override() {
    use std::sync::Arc;
    use tinyagents::harness::message::Message;
    use tinyagents::harness::model::ModelRequest;
    use tinyagents::harness::testkit::ScriptedModel;

    let _guard = crate::openhuman::inference::inference_test_guard();

    // The factory consults this override under cfg(test), so `create_chat_model`
    // resolves to the mock without needing configured cloud providers.
    let _override = test_provider_override::install_model(Arc::new(ScriptedModel::replies(vec![
        "echo: hi there",
    ])));
    let config = Config::default();

    let model = create_chat_model("chat", &config, 0.3).expect("create_chat_model must build");
    let response = model
        .invoke(&(), ModelRequest::new(vec![Message::user("hi there")]))
        .await
        .expect("invoke must succeed");
    assert_eq!(response.text(), "echo: hi there");
}

#[tokio::test]
async fn one_shot_chat_models_preserve_factory_temperature_as_request_default() {
    use async_trait::async_trait;
    use std::sync::{Arc, Mutex};
    use tinyagents::harness::message::Message;
    use tinyagents::harness::model::{ModelRequest, ModelResponse};

    struct TemperatureProbe {
        seen: Arc<Mutex<Vec<Option<f64>>>>,
    }

    #[async_trait]
    impl ChatModel<()> for TemperatureProbe {
        async fn invoke(
            &self,
            _state: &(),
            request: ModelRequest,
        ) -> tinyagents::Result<ModelResponse> {
            self.seen
                .lock()
                .expect("probe lock")
                .push(request.temperature);
            Ok(ModelResponse::assistant("ok"))
        }
    }

    let _guard = crate::openhuman::inference::inference_test_guard();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let _override = test_provider_override::install_model(Arc::new(TemperatureProbe {
        seen: Arc::clone(&seen),
    }));

    let config = Config::default();
    let role_model = create_chat_model("chat", &config, 0.3).expect("role model");
    role_model
        .invoke(&(), ModelRequest::new(vec![Message::user("default")]))
        .await
        .expect("default-temperature invoke");

    let explicit_model = create_chat_model_from_string("chat", "openhuman", &config, 0.7)
        .expect("explicit provider model");
    explicit_model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("explicit")]).with_temperature(0.9),
        )
        .await
        .expect("explicit-temperature invoke");

    let turn_model = create_turn_chat_model("chat", &config, "chat-v1", 0.2).expect("turn model");
    turn_model
        .invoke(&(), ModelRequest::new(vec![Message::user("turn default")]))
        .await
        .expect("turn default-temperature invoke");

    let explicit_turn_model =
        create_turn_chat_model_from_string("chat", "openhuman", &config, "chat-v1", 0.4)
            .expect("explicit turn model");
    explicit_turn_model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("turn explicit")]).with_temperature(0.8),
        )
        .await
        .expect("turn explicit-temperature invoke");

    assert_eq!(
        *seen.lock().expect("probe lock"),
        vec![Some(0.3), Some(0.9), Some(0.2), Some(0.8)]
    );
}

// ── Motion B (#4727): managed-backend crate-native routing ──────────────────
// `create_chat_model` must route the managed OpenHuman backend through the
// crate-native `OpenHumanBackendModel`, whose concrete `managed` profile
// advertises the capabilities that routing previously inferred through the
// provider adapter.

#[test]
fn resolves_to_managed_backend_for_default_config_but_not_for_local() {
    // A default config has no BYOK/cloud providers, so every chat-tier role
    // resolves to the managed OpenHuman backend.
    let managed = Config::default();
    assert!(resolves_to_managed_backend("chat", &managed));
    assert!(resolves_to_managed_backend("reasoning", &managed));

    // Pointing the chat role at a local runtime opts it out of the managed path.
    let mut local = Config::default();
    local.chat_provider = Some("ollama:qwen2.5".to_string());
    assert!(!resolves_to_managed_backend("chat", &local));
}

#[test]
fn create_chat_model_routes_managed_backend_to_crate_native() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // No test-provider override installed → the managed short-circuit engages.
    let config = Config::default();
    let (model, _model_id) = create_chat_model_with_model_id("chat", &config, 0.7)
        .expect("managed create_chat_model must build");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("managed"),
        "managed backend must expose the crate-native managed profile"
    );
}

#[test]
fn create_chat_model_routes_local_runtime_to_crate_native() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.chat_provider = Some("ollama:qwen2.5".to_string());
    let (model, model_id) = create_chat_model_with_model_id("chat", &config, 0.7)
        .expect("local create_chat_model must build");
    assert_eq!(model_id, "qwen2.5");
    // Motion B (#4727): a local runtime now builds a crate-native `OpenAiModel`
    // (not a legacy model wrapper), so its profile carries the concrete
    // provider slug — `ollama`, not the adapter's neutral `local`/`remote` — and
    // native tools + vision are forced off (Ollama rejects the OpenAI `tools`
    // param and is text-only here).
    let profile = model
        .profile()
        .expect("crate-native local model exposes a profile");
    assert_eq!(profile.provider.as_deref(), Some("ollama"));
    assert!(!profile.tool_calling, "Ollama disables native tool calling");
    assert!(!profile.modalities.image_in, "Ollama is text-only here");
}

#[test]
fn explicit_local_provider_string_routes_to_crate_native_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = Config::default();
    let (model, model_id) =
        create_chat_model_from_string_with_model_id("chat", "ollama:qwen2.5", &config, 0.7)
            .expect("explicit local model must build");
    assert_eq!(model_id, "qwen2.5");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("ollama")
    );
}

#[test]
fn try_create_local_runtime_returns_none_for_managed_and_cloud() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // Default config resolves to the managed backend, not a local runtime.
    assert!(try_create_local_runtime_chat_model("chat", &Config::default()).is_none());
    // A BYOK cloud slug is not a local runtime either — it falls through to the
    // `Provider` path.
    let mut cloud = Config::default();
    cloud.cloud_providers.push(openai_entry("p_oai", "openai"));
    cloud.chat_provider = Some("openai:gpt-4o-mini".to_string());
    assert!(try_create_local_runtime_chat_model("chat", &cloud).is_none());
}

// ── Motion B (#4727 Phase 3): wire-equivalent BYOK cloud-slug cutover ────────

fn deepseek_entry(id: &str) -> CloudProviderCreds {
    CloudProviderCreds {
        id: id.to_string(),
        slug: "deepseek".to_string(),
        label: "DeepSeek".to_string(),
        endpoint: "https://api.deepseek.com/v1".to_string(),
        auth_style: AuthStyle::Bearer,
        default_model: Some("deepseek-chat".to_string()),
        ..Default::default()
    }
}

#[test]
fn create_chat_model_routes_plain_bearer_cloud_slug_to_crate_native() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // DeepSeek is a built-in chat-completions-only Bearer provider: no
    // `/v1/responses` fallback and no codex-oauth, so it is wire-equivalent and
    // flips crate-native.
    let mut config = Config::default();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    config.chat_provider = Some("deepseek:deepseek-reasoner".to_string());
    let (model, model_id) = create_chat_model_with_model_id("chat", &config, 0.7)
        .expect("bearer cloud create_chat_model must build");
    assert_eq!(model_id, "deepseek-reasoner");
    let profile = model
        .profile()
        .expect("crate-native cloud model exposes a profile");
    assert_eq!(profile.provider.as_deref(), Some("deepseek"));
    // A generic cloud model keeps native tool calling + vision on (unlike the
    // local runtimes), so this is the crate `OpenAiModel` default profile.
    assert!(profile.tool_calling);
}

#[test]
fn turn_model_route_metadata_uses_post_remap_cloud_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    config.chat_provider = Some("deepseek:chat-v1".to_string());

    let (_model, provider, resolved_model) =
        create_turn_chat_model_with_native_tools_and_route("chat", &config, "chat-v1", 0.7, true)
            .expect("abstract BYOK tier must build");

    assert_eq!(provider, "deepseek");
    assert_eq!(resolved_model, "deepseek-chat");
}

#[test]
fn explicit_cloud_provider_string_routes_to_crate_native_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    let (model, model_id) = create_chat_model_from_string_with_model_id(
        "chat",
        "deepseek:deepseek-reasoner",
        &config,
        0.7,
    )
    .expect("explicit cloud model must build");
    assert_eq!(model_id, "deepseek-reasoner");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("deepseek")
    );
}

#[test]
fn create_chat_model_routes_anthropic_auth_cloud_slug_to_crate_native() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // Anthropic-auth cloud slugs are always wire-equivalent (their endpoints have
    // no `/v1/responses`, so the host's dormant fallback is behavior-neutral).
    let mut config = Config::default();
    config
        .cloud_providers
        .push(anthropic_entry("p_anth", "anthropic"));
    config.chat_provider = Some("anthropic:claude-sonnet-4-6".to_string());
    let (model, model_id) = create_chat_model_with_model_id("chat", &config, 0.7)
        .expect("anthropic cloud create_chat_model must build");
    assert_eq!(model_id, "claude-sonnet-4-6");
    assert_eq!(
        model.profile().and_then(|p| p.provider.as_deref()),
        Some("anthropic")
    );
}

#[test]
fn configured_openhuman_jwt_slug_routes_to_managed_chat_model() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));
    config.chat_provider = Some("openhuman:reasoning-v1".to_string());

    let (model, model_id) = try_create_cloud_slug_chat_model("chat", &config)
        .expect("configured OpenhumanJwt slug should be recognized")
        .expect("managed model should build");

    assert_eq!(model_id, "reasoning-v1");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("managed"),
        "OpenhumanJwt must use the crate-native managed backend model"
    );
}

#[tokio::test]
async fn openhuman_jwt_slug_discloses_pinned_model() {
    use crate::core::event_bus::{init_global, publish_global, DomainEvent, DEFAULT_CAPACITY};
    use crate::openhuman::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    let _guard = crate::openhuman::inference::inference_test_guard();
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let marker = "egress-jwt-pinned-marker-v1";
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));
    let provider = format!("openhuman:{marker}");
    let _ = try_create_cloud_slug_chat_model_from_string("chat", &provider, &config)
        .expect("configured OpenhumanJwt slug should be recognized")
        .expect("managed model should build");

    let sentinel = "egress-jwt-pinned-sentinel-end";
    publish_global(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
    });

    let mut count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                if descriptor.service == marker {
                    assert_eq!(descriptor.provider_slug, "openhuman");
                    assert!(matches!(descriptor.reason, EgressReason::Inference));
                    count += 1;
                } else if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(Ok(_)) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("event bus closed before sentinel arrived")
            }
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
    assert_eq!(
        count, 1,
        "JWT construction must disclose its pinned model once"
    );
}

#[tokio::test]
async fn native_claude_turn_routes_disclose_pinned_models() {
    use crate::core::event_bus::{init_global, publish_global, DomainEvent, DEFAULT_CAPACITY};
    use crate::openhuman::security::egress::EgressDescriptor;
    use std::time::Duration;

    let _guard = crate::openhuman::inference::inference_test_guard();
    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let configured_sdk = "egress-sdk-configured-marker";
    let pinned_sdk = "egress-sdk-pinned-marker";
    let mut sdk_config = Config::default();
    sdk_config.chat_provider = Some(format!("claude_agent_sdk:{configured_sdk}"));
    create_turn_chat_model("chat", &sdk_config, pinned_sdk, 0.0)
        .expect("Claude Agent SDK turn model should build");

    let configured_code = "egress-code-configured-marker";
    let pinned_code = "egress-code-pinned-marker";
    let mut code_config = Config::default();
    code_config.chat_provider = Some(format!("claude-code:{configured_code}"));
    // Egress is disclosed once the effective model is selected, before the
    // environment probe. The test therefore remains valid on hosts without the
    // Claude Code CLI.
    let _ = create_turn_chat_model("chat", &code_config, pinned_code, 0.0);

    let sentinel = "egress-native-claude-sentinel-end";
    publish_global(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
    });

    let mut sdk_count = 0usize;
    let mut code_count = 0usize;
    let mut configured_count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                match descriptor.service.as_str() {
                    service if service == pinned_sdk => {
                        assert_eq!(descriptor.provider_slug, "claude_agent_sdk");
                        sdk_count += 1;
                    }
                    service if service == pinned_code => {
                        assert_eq!(descriptor.provider_slug, "claude-code");
                        code_count += 1;
                    }
                    service if service == configured_sdk || service == configured_code => {
                        configured_count += 1;
                    }
                    service if service == sentinel => break,
                    _ => {}
                }
            }
            Ok(Ok(_)) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("event bus closed before sentinel arrived")
            }
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }

    assert_eq!(
        sdk_count, 1,
        "SDK route must disclose its pinned model once"
    );
    assert_eq!(
        code_count, 1,
        "Claude Code route must disclose its pinned model once"
    );
    assert_eq!(
        configured_count, 0,
        "native Claude routes must not disclose stale configured models"
    );
}

#[test]
fn openhuman_jwt_slug_preserves_forced_text_mode() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));

    let (model, _) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        "chat",
        "openhuman:reasoning-v1",
        &config,
        false,
    )
    .expect("configured OpenhumanJwt slug should be recognized")
    .expect("managed model should build");

    let profile = model
        .profile()
        .expect("managed model should expose its effective capabilities");
    assert!(!profile.tool_calling);
    assert!(!profile.parallel_tool_calls);
    assert!(!profile.streaming_tool_chunks);
}

#[test]
fn openhuman_jwt_slug_without_model_preserves_managed_role_tier() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));

    let (_model, model_id) =
        try_create_cloud_slug_chat_model_from_string("summarization", "openhuman:", &config)
            .expect("configured OpenhumanJwt slug should be recognized")
            .expect("managed model should build");

    assert_eq!(model_id, crate::openhuman::config::MODEL_SUMMARIZATION_V1);
}

#[test]
fn try_create_cloud_slug_flips_openai_but_declines_non_cloud() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    // `openai` (API-key Bearer, no codex OAuth) now flips crate-native on Chat
    // Completions — the legacy `/v1/responses` fallback is not replicated.
    let mut openai = Config::default();
    openai.cloud_providers.push(openai_entry("p_oai", "openai"));
    openai.chat_provider = Some("openai:gpt-4o-mini".to_string());
    let (model, model_id) = try_create_cloud_slug_chat_model("chat", &openai)
        .expect("openai should flip crate-native")
        .expect("build");
    assert_eq!(model_id, "gpt-4o-mini");
    assert_eq!(
        model.profile().and_then(|p| p.provider.as_deref()),
        Some("openai")
    );

    // Managed (default), local runtimes, and unconfigured slugs are not cloud
    // slugs — they decline and fall through to their own paths.
    assert!(try_create_cloud_slug_chat_model("chat", &Config::default()).is_none());
    let mut local = Config::default();
    local.chat_provider = Some("ollama:qwen2.5".to_string());
    assert!(try_create_cloud_slug_chat_model("chat", &local).is_none());
    let mut unconfigured = Config::default();
    unconfigured.chat_provider = Some("deepseek:deepseek-chat".to_string());
    assert!(try_create_cloud_slug_chat_model("chat", &unconfigured).is_none());
}

#[test]
fn crate_native_chat_model_factory_preserves_invalid_route_diagnostics() {
    let _guard = crate::openhuman::inference::inference_test_guard();
    let config = Config::default();

    let unconfigured =
        create_chat_model_from_string_with_model_id("reasoning", "groq:llama3", &config, 0.7)
            .err()
            .expect("unconfigured slug must fail")
            .to_string();
    assert!(
        unconfigured.contains("no cloud provider configured for slug 'groq'"),
        "unexpected diagnostic: {unconfigured}"
    );

    let bare =
        create_chat_model_from_string_with_model_id("reasoning", "unknown-provider", &config, 0.7)
            .err()
            .expect("bare unknown provider must fail")
            .to_string();
    assert!(
        bare.contains("unrecognised provider string 'unknown-provider'"),
        "unexpected diagnostic: {bare}"
    );

    let byok = create_chat_model_from_string_with_model_id(
        "reasoning",
        BYOK_INCOMPLETE_SENTINEL,
        &config,
        0.7,
    )
    .err()
    .expect("incomplete BYOK must fail")
    .to_string();
    assert!(
        byok.contains("BYOK_INCOMPLETE"),
        "unexpected diagnostic: {byok}"
    );
}

/// Real-path smoke (privacy epic S2, #4436): driving the actual inference
/// chokepoint `create_test_chat_model_from_string` with an EXTERNAL provider must
/// publish an `ExternalTransferPending` egress event — proving the emit is wired
/// into the live construction path, not merely callable in isolation.
/// Complements the isolated emit unit tests in `security::egress`.
#[tokio::test]
async fn from_string_external_provider_emits_egress_realpath() {
    use crate::core::event_bus::{init_global, DomainEvent, DEFAULT_CAPACITY};
    use crate::openhuman::security::egress::EgressReason;

    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let config = Config::default();
    // External provider → real chokepoint must emit BEFORE constructing.
    let _ = create_test_chat_model_from_string("agentic", "openai:gpt-4o-mini", &config);

    // Bus is process-wide; drain past unrelated events until our descriptor lands.
    let found = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Ok(DomainEvent::ExternalTransferPending { descriptor, .. })
                    if descriptor.provider_slug == "openai"
                        && descriptor.is_external
                        && matches!(descriptor.reason, EgressReason::Inference) =>
                {
                    return descriptor;
                }
                Ok(_) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    panic!("event bus closed before ExternalTransferPending arrived")
                }
            }
        }
    })
    .await;

    assert!(
        found.is_ok(),
        "external inference via create_test_chat_model_from_string must publish ExternalTransferPending"
    );
}

/// Real-path smoke (privacy epic S2, #4436): the crate-native ChatModel path
/// `create_chat_model_with_model_id` — the path production agent turns use
/// post-#4784 — must emit EXACTLY ONE egress descriptor for a managed-backend
/// (external) construction. Regression guard for the gap where emit lived only
/// on the legacy `Provider` path, so the default managed turn disclosed nothing.
#[tokio::test]
async fn create_chat_model_managed_emits_exactly_one_egress_realpath() {
    use crate::core::event_bus::{init_global, publish_global, DomainEvent, DEFAULT_CAPACITY};
    use crate::openhuman::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    // Unique model marker so the process-wide bus can't confuse a concurrent
    // test's managed event with ours. `heartbeat` has no managed tier and
    // resolves to the managed backend, so `default_model` flows through verbatim.
    let marker = "egress-managed-realpath-marker-v1";
    let mut config = Config::default();
    config.default_model = Some(marker.to_string());
    let _ = create_chat_model_with_model_id("heartbeat", &config, 0.7);

    // Bound the drain with a unique sentinel published AFTER our construction.
    let sentinel = "egress-managed-sentinel-end";
    publish_global(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
    });

    let mut count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                if descriptor.service == marker {
                    assert_eq!(descriptor.provider_slug, "openhuman");
                    assert!(descriptor.is_external, "managed backend is external");
                    assert!(matches!(descriptor.reason, EgressReason::Inference));
                    count += 1;
                } else if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(Ok(_)) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("event bus closed before sentinel arrived")
            }
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
    assert_eq!(
        count, 1,
        "managed inference via create_chat_model_with_model_id must emit EXACTLY ONE egress descriptor (no miss, no double)"
    );
}

/// Real-path smoke (privacy epic S2, #4436): a LOCAL runtime construction on the
/// crate-native ChatModel path must NOT publish an `ExternalTransferPending`
/// (nothing leaves the device — it is disclosed as non-external, no event).
#[tokio::test]
async fn create_chat_model_local_runtime_does_not_emit_egress_realpath() {
    use crate::core::event_bus::{init_global, publish_global, DomainEvent, DEFAULT_CAPACITY};
    use crate::openhuman::security::egress::EgressDescriptor;
    use std::time::Duration;

    init_global(DEFAULT_CAPACITY);
    let mut rx = crate::core::event_bus::global().unwrap().raw_receiver();

    let local_marker = "egress-local-realpath-marker";
    let mut config = Config::default();
    config.chat_provider = Some(format!("ollama:{local_marker}"));
    let _ = create_chat_model_with_model_id("chat", &config, 0.7);

    // Sentinel bounds the drain; if the local marker ever appears as an external
    // transfer before it, the local-suppression contract is broken.
    let sentinel = "egress-local-sentinel-end";
    publish_global(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
    });

    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Ok(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                assert_ne!(
                    descriptor.service, local_marker,
                    "local runtime must NOT publish ExternalTransferPending"
                );
                if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(Ok(_)) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => continue,
            Ok(Err(tokio::sync::broadcast::error::RecvError::Closed)) => {
                panic!("event bus closed before sentinel arrived")
            }
            Err(_) => panic!("timed out before egress sentinel arrived"),
        }
    }
}

// ─── #5146 §2.1: local chat + background workloads ────────────────────────────
//
// The fix for #5146 §2.1 is an *explanation* change, not a routing change. These
// pin the routing so a future "never fall back" refactor cannot silently break
// local-chat + managed-subscription users without a failing test.

#[test]
fn local_chat_still_routes_background_roles_to_the_managed_backend() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // Every background role keeps falling through to the managed backend: they
    // run tier-specific models a local runtime does not serve, and the user's
    // subscription is what pays for them.
    for role in [
        "vision",
        "embeddings",
        "memory",
        "summarization",
        "heartbeat",
        "learning",
        "subconscious",
        "agentic",
        "burst",
    ] {
        assert_eq!(
            provider_for_role(role, &config),
            "openhuman",
            "role '{role}' must keep falling back to the managed backend when chat is local"
        );
    }
}

#[test]
fn local_chat_role_is_returned_verbatim_and_never_falls_back() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    assert_eq!(
        provider_for_role("chat", &config),
        "ollama:gemma3:1b",
        "an explicitly configured local chat route must be honoured verbatim"
    );
}

#[test]
fn explicit_background_route_overrides_the_cloud_fallback() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());
    config.vision_provider = Some("ollama:llava:7b".to_string());

    // The remedy the new diagnostics point users at must actually work.
    assert_eq!(
        provider_for_role("vision", &config),
        "ollama:llava:7b",
        "setting vision_provider must take precedence over the cloud fallback"
    );
}

#[test]
fn a_readable_profile_with_no_stored_key_is_treated_as_missing_credentials() {
    // The common BYOK-with-no-key shape: the auth profile reads fine, it just
    // has nothing for this slug, so the lookup succeeds with an empty string.
    // Without an emptiness check the client would be built with a blank bearer
    // and the user would get a raw 401 from the provider instead of guidance.
    let _guard = crate::openhuman::inference::inference_test_guard();
    let tmp = TempDir::new().expect("tempdir");
    let mut config = config_with_providers_in_tempdir(&tmp, vec![openai_entry("p_oai", "openai")]);
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    let err = create_test_chat_model_from_string("vision", "openai:gpt-4o", &config)
        .err()
        .expect("a slug with no stored key must not build a client");
    let message = err.to_string();

    assert!(
        message.contains("No usable credentials for 'openai'"),
        "expected the actionable guidance, got: {message}"
    );
    // It is a genuine implicit fallback here (vision has no route of its own),
    // so the local chat model that caused it is named.
    assert!(
        message.contains("ollama:gemma3:1b"),
        "expected the local chat model to be named, got: {message}"
    );

    // Scope: an explicitly routed provider is NOT failed at construction time
    // for a missing key. Callers build such models to probe or describe a
    // provider before a key is saved, so only the implicit-fallback path (the
    // one this diagnostic exists for) turns a blank key into an error.
    config.vision_provider = Some("openai:gpt-4o".to_string());
    assert!(
        create_test_chat_model_from_string("vision", "openai:gpt-4o", &config).is_ok(),
        "an explicitly routed provider must still build without a stored key"
    );
}

#[test]
fn implicit_cloud_fallback_is_claimed_only_when_the_role_has_no_route_of_its_own() {
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // Unset background route: the role genuinely landed on the cloud because
    // the local chat model cannot serve it, so the explanation applies.
    assert!(role_uses_implicit_cloud_fallback("vision", &config));
    // The literal "cloud" is the same "route me wherever the cloud is" intent.
    config.embeddings_provider = Some("cloud".to_string());
    assert!(role_uses_implicit_cloud_fallback("embeddings", &config));
    // Whitespace is not a configured route.
    config.memory_provider = Some("   ".to_string());
    assert!(role_uses_implicit_cloud_fallback("memory", &config));

    // Explicitly routed to a cloud slug: a credential failure here is about
    // that route, not about the local chat model, so it must not be described
    // as a fallback.
    config.vision_provider = Some("anthropic:claude-3-5-sonnet-latest".to_string());
    assert!(!role_uses_implicit_cloud_fallback("vision", &config));

    // Chat-tier roles are never described as cloud fallbacks, routed or not.
    for role in ["chat", "reasoning", "coding"] {
        assert!(!role_uses_implicit_cloud_fallback(role, &config));
    }
}

#[test]
fn cloud_fallback_roles_match_the_roles_provider_for_role_actually_falls_back() {
    // `factory_tests` is a child module of `factory`, so `super` is `factory`,
    // not `provider` — reach the sibling module by its crate path.
    use crate::openhuman::inference::provider::fallback_diagnostics::role_falls_back_to_cloud;
    let mut config = Config::default();
    config.chat_provider = Some("ollama:gemma3:1b".to_string());

    // The diagnostics module carries its own role list; if the two drift, users
    // get either a missing explanation or one that names the wrong knob.
    for role in [
        "vision",
        "embeddings",
        "memory",
        "summarization",
        "heartbeat",
        "learning",
        "subconscious",
        "agentic",
        "burst",
    ] {
        assert!(
            role_falls_back_to_cloud(role),
            "'{role}' falls back in provider_for_role but is missing from CLOUD_FALLBACK_ROLES"
        );
    }
    for role in ["chat", "reasoning", "coding"] {
        assert!(
            !role_falls_back_to_cloud(role),
            "'{role}' is a chat-tier role and must not be described as a cloud fallback"
        );
    }
}
