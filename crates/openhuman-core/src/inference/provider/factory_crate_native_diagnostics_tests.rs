use super::*;

use tinyinference_llm::providers::registry::openrouter_default_provider_options;

#[test]
fn crate_native_chat_model_factory_preserves_invalid_route_diagnostics() {
    let _guard = crate::inference::inference_test_guard();
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
    use crate::core::events::DomainEvent;
    use crate::security::egress::EgressReason;

    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let config = Config::default();
    // External provider → real chokepoint must emit BEFORE constructing.
    let _ = create_test_chat_model_from_string("agentic", "openai:gpt-4o-mini", &config);

    // Bus is process-wide; drain past unrelated events until our descriptor lands.
    let found = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            match rx.recv().await {
                Some(DomainEvent::ExternalTransferPending { descriptor, .. })
                    if descriptor.provider_slug == "openai"
                        && descriptor.is_external
                        && matches!(descriptor.reason, EgressReason::Inference) =>
                {
                    return descriptor;
                }
                Some(_) => continue,
                None => panic!("the bus closed before the expected event arrived"),
            }
        }
    })
    .await;

    assert!(
        found.is_ok(),
        "external inference via create_test_chat_model_from_string must publish ExternalTransferPending"
    );
}

#[tokio::test]
async fn caller_owned_models_build_without_openhuman_session() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        config_path: dir.path().join("config.toml"),
        workspace_dir: dir.path().join("workspace"),
        ..Config::default()
    };
    for provider in [
        "ollama:test-model",
        "lmstudio:test-model",
        "mlx:test-model",
        "omlx:test-model",
        "local-openai:test-model",
        "claude_agent_sdk:test-model",
    ] {
        create_chat_model_from_string("chat", provider, &config, 0.0)
            .unwrap_or_else(|e| panic!("{provider} must build while signed out: {e}"));
    }
    // CLI discovery is machine-specific; verify its authentication gate without
    // starting or requiring an installed CLI.
    crate::inference::provider::factory::access_gates::verify_provider_session(
        &config,
        "claude-code:test-model",
    )
    .expect("Claude Code uses its own authentication while OpenHuman is signed out");
    // Exercise the real gate (cloud constructors skip auth under cfg(test)).
    for provider in [
        "openhuman",
        "openai:test-model",
        "unknown:test-model",
        "cloud",
    ] {
        let error = crate::inference::provider::factory::access_gates::verify_provider_session(
            &config, provider,
        )
        .unwrap_err();
        assert!(
            error.to_string().contains("SESSION_EXPIRED"),
            "{provider}: {error}"
        );
    }
}
#[tokio::test]
async fn local_aliases_build_without_a_session_and_preserve_model_ids() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);
    let config = Config::default();
    for (prefix, canonical) in [
        ("OLLAMA", "ollama"),
        ("LMSTUDIO", "lmstudio"),
        ("lm-studio", "lmstudio"),
        ("lm_studio", "lmstudio"),
        ("MLX", "mlx"),
        ("OMLX", "omlx"),
        ("LOCAL-OPENAI", "local-openai"),
        ("local_openai", "local-openai"),
    ] {
        let provider = format!(" {prefix}:Publisher/Model:Tag@0.4 ");
        let (chat, model) =
            create_chat_model_from_string_with_model_id("chat", &provider, &config, 0.0)
                .unwrap_or_else(|e| panic!("{provider}: {e}"));
        assert_eq!(model, "Publisher/Model:Tag");
        assert_eq!(
            chat.profile().and_then(|p| p.provider.as_deref()),
            Some(canonical)
        );
    }
    // A bare provider names a runtime but not a model. Report that actual
    // configuration problem instead of incorrectly asking for a session.
    let error = create_chat_model_from_string("chat", "ollama", &config, 0.0)
        .err()
        .expect("bare Ollama must require a model ID");
    assert!(error.to_string().contains("empty model"), "{error}");
    assert!(!error.to_string().contains("SESSION_EXPIRED"), "{error}");
}

#[test]
fn direct_openrouter_endpoints_get_price_sorted_routing_and_nothing_else() {
    // Direct BYOK OpenRouter: cheapest-first sort so consecutive turns stay on
    // one endpoint and its prefix cache hits. Only the sort — never `order` or
    // `allow_fallbacks: false`, which would strand a request on an outage, and
    // never a `max_price` (the hosted backend dropped its own for the same
    // reason in tinyhumansai/backend#1370).
    let options = openrouter_default_provider_options("https://openrouter.ai/api/v1")
        .expect("openrouter endpoint carries routing options");
    assert_eq!(
        options,
        serde_json::json!({ "provider": { "sort": "price" } }),
        "exactly the sort and nothing else: {options}"
    );
    let provider = &options["provider"];
    for forbidden in ["order", "allow_fallbacks", "max_price", "only", "ignore"] {
        assert!(
            provider.get(forbidden).is_none(),
            "must not set provider.{forbidden}"
        );
    }
    // Host matching is what keys it, with or without a path or trailing slash.
    assert!(openrouter_default_provider_options("https://openrouter.ai/api/v1/").is_some());
    assert!(openrouter_default_provider_options("https://OpenRouter.ai/api/v1").is_some());
}

#[test]
fn non_openrouter_openai_compatible_endpoints_get_no_baked_provider_options() {
    // `provider` is an OpenRouter-only body field; hosted OpenAI rejects
    // unknown top-level fields and local runners ignore them, so no other
    // OpenAI-compatible host may receive it.
    for endpoint in [
        "https://api.openai.com/v1",
        "https://api.deepseek.com/v1",
        "https://api.groq.com/openai/v1",
        "http://localhost:11434/v1",
        "https://openrouter.example.com/v1",
        "not a url",
    ] {
        assert!(
            openrouter_default_provider_options(endpoint).is_none(),
            "{endpoint} must not get OpenRouter routing options"
        );
    }
}
