use super::*;

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
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference::message::Message;
    use tinyinference::model::ModelRequest;

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
    use tinyinference::message::Message;
    use tinyinference::model::{ModelRequest, ModelResponse};

    struct TemperatureProbe {
        seen: Arc<Mutex<Vec<Option<f64>>>>,
    }

    #[async_trait]
    impl ChatModel<()> for TemperatureProbe {
        async fn invoke(
            &self,
            _state: &(),
            request: ModelRequest,
        ) -> tinyinference::Result<ModelResponse> {
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
    use crate::core::events::DomainEvent;
    use crate::openhuman::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    let _guard = crate::openhuman::inference::inference_test_guard();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "egress-jwt-pinned-marker-v1";
    let mut config = Config::default();
    config.cloud_providers.push(oh_entry("p_oh"));
    let provider = format!("openhuman:{marker}");
    let _ = try_create_cloud_slug_chat_model_from_string("chat", &provider, &config)
        .expect("configured OpenhumanJwt slug should be recognized")
        .expect("managed model should build");

    let sentinel = "egress-jwt-pinned-sentinel-end";
    crate::core::bus::BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
    });

    let mut count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
                if descriptor.service == marker {
                    assert_eq!(descriptor.provider_slug, "openhuman");
                    assert!(matches!(descriptor.reason, EgressReason::Inference));
                    count += 1;
                } else if descriptor.service == sentinel {
                    break;
                }
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
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
    use crate::core::bus::BUS;
    use crate::core::events::DomainEvent;
    use crate::openhuman::security::egress::EgressDescriptor;
    use std::time::Duration;

    let _guard = crate::openhuman::inference::inference_test_guard();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

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
    BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
    });

    let mut sdk_count = 0usize;
    let mut code_count = 0usize;
    let mut configured_count = 0usize;
    loop {
        match tokio::time::timeout(Duration::from_secs(3), rx.recv()).await {
            Ok(Some(DomainEvent::ExternalTransferPending { descriptor, .. })) => {
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
            Ok(Some(_)) => continue,
            Ok(None) => panic!("the bus closed before the sentinel arrived"),
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
    use crate::core::events::DomainEvent;
    use crate::openhuman::security::egress::EgressReason;

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
