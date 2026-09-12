use super::*;

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

// #6109 removed `resolve_byok_fallback_provider_string` along with the chat-tier
// inheritance it fed. These two cases previously asserted that *local* providers
// were not eligible to be inherited; they now assert the stronger property that
// replaced it — an unset route inherits nothing at all — through the public seam.

#[test]
fn unset_route_does_not_inherit_a_local_sibling() {
    let mut config = Config::default();
    config.chat_provider = Some("mlx:llama3".to_string());
    config.reasoning_provider = Some("local-openai:phi3".to_string());
    assert_eq!(
        provider_for_role("coding", &config),
        "openhuman",
        "an unset coding route must fall through to the managed backend"
    );
}

#[test]
fn unset_route_does_not_inherit_omlx() {
    let mut config = Config::default();
    config.chat_provider = Some("omlx:llama3".to_string());
    assert_eq!(
        provider_for_role("coding", &config),
        "openhuman",
        "unset coding must not pick up chat's OMLX route"
    );
}

/// The #6109 regression itself: a user who configures exactly one chat-tier
/// route must not find the other two moved onto it. Before the fix,
/// `coding_provider` alone re-routed both `chat` and `reasoning` to
/// `openrouter:zai/glm-4.7` — their ordinary conversations billed to their own
/// key, with no settings field saying so.
#[test]
fn setting_one_chat_tier_route_leaves_its_siblings_on_the_managed_backend() {
    let mut config = Config::default();
    config.coding_provider = Some("openrouter:zai/glm-4.7".to_string());

    assert_eq!(
        provider_for_role("coding", &config),
        "openrouter:zai/glm-4.7",
        "the route the user actually set must be honoured"
    );
    for sibling in ["chat", "reasoning"] {
        assert_eq!(
            provider_for_role(sibling, &config),
            "openhuman",
            "`{sibling}` was never configured and must stay on the managed backend, \
             not inherit the coding route"
        );
    }
}

/// The relation was not even symmetric before the fix: `agentic_provider` was
/// read as a donor but never received a route. Both directions are now inert.
#[test]
fn agentic_route_is_neither_donor_nor_beneficiary() {
    let mut config = Config::default();
    config.agentic_provider = Some("openrouter:some/model".to_string());
    assert_eq!(
        provider_for_role("chat", &config),
        "openhuman",
        "chat must not inherit the agentic route"
    );

    let mut config = Config::default();
    config.chat_provider = Some("openrouter:some/model".to_string());
    assert_eq!(
        provider_for_role("agentic", &config),
        "openhuman",
        "agentic must not inherit the chat route"
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
