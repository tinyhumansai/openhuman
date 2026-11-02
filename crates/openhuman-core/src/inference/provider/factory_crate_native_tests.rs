use super::super::cloud_slug::try_create_cloud_slug_chat_model_from_string;
use super::*;
use crate::security::egress::EgressLedger;

use crate::inference::provider::factory::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools;

fn isolated_config() -> (tempfile::TempDir, Config) {
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    (tmp, config)
}

async fn store_fixture_session(config: &Config) {
    crate::credentials::AuthService::from_config(config)
        .store_provider_token(
            crate::credentials::APP_SESSION_PROVIDER,
            "default",
            "fixture-session-token",
            Default::default(),
            true,
        )
        .await
        .expect("store isolated fixture session");
}

async fn store_fixture_cloud_key(config: &Config, slug: &str) {
    crate::credentials::AuthService::from_config(config)
        .store_provider_token(
            &format!("provider:{slug}"),
            "default",
            "sk-fixture-provider",
            Default::default(),
            true,
        )
        .await
        .expect("store isolated provider credential");
}

fn fixture_library_dispatch() -> crate::core::runtime::DispatchCtx {
    crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_host_kind(
            crate::core::types::HostKind::Library,
            crate::core::runtime::DomainSet::full(),
            None,
        ),
    )
}

async fn with_fixture_library_context<F: std::future::Future>(f: impl FnOnce() -> F) -> F::Output {
    let context = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Library,
        crate::core::runtime::DomainSet::full(),
        None,
    );
    crate::core::runtime::CoreContext::scope(context, f()).await
}

#[tokio::test]
async fn custom_ca_bundle_builds_a_provider_scoped_chat_client() {
    let _guard = crate::inference::inference_test_guard();
    let (_tmp, mut config) = isolated_config();
    config.cloud_providers.push(openai_entry("p_team", "team"));
    store_fixture_cloud_key(&config, "team").await;
    config.cloud_provider_ca_certs.insert(
        "team".into(),
        include_str!("../../util/tls/test-ca.pem").into(),
    );
    let (model, model_id) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        "chat",
        "team:private-model",
        &config,
        true,
        &EgressLedger::none(),
    )
    .await
    .expect("configured provider")
    .expect("custom CA client should build");
    assert_eq!(model_id, "private-model");
    assert_eq!(
        model.profile().and_then(|profile| profile.provider.clone()),
        Some("team".into())
    );
}
#[tokio::test]
async fn enforce_local_only_inference_errors_on_external_when_local_only() {
    // Drive the live-policy-backed wrapper: install a LocalOnly policy, then
    // assert an external provider is refused with the privacy message and a
    // local provider passes. Factory tests use `inference_test_guard`; take the
    // same lock before mutating the process-global live policy so parallel
    // cloud-model construction cannot observe this temporary LocalOnly mode.
    let _inference = crate::inference::inference_test_guard();
    let _env = crate::config::TEST_ENV_LOCK.blocking_lock();
    use crate::config::PrivacyMode;
    use crate::security::SecurityPolicy;
    let ws = std::env::temp_dir().join("openhuman_factory_privacy_test");
    let policy = std::sync::Arc::new(
        SecurityPolicy {
            workspace_dir: ws.clone(),
            ..SecurityPolicy::default()
        }
        .with_privacy_mode(PrivacyMode::LocalOnly),
    );
    crate::security::live_policy::install(policy, ws.clone(), ws.clone());

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
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("direct Claude SDK model must preserve the privacy gate"),
    };
    assert!(sdk_error.to_string().contains("Local-only privacy mode"));

    let claude_code_error = match create_chat_model_from_string(
        "coding",
        "claude-code:claude-sonnet-4-6",
        &Config::default(),
        0.0,
    )
    .await
    {
        Err(error) => error,
        Ok(_) => panic!("direct Claude Code model must preserve the privacy gate"),
    };
    assert!(claude_code_error
        .to_string()
        .contains("Local-only privacy mode"));

    // Both the default factory and a thread-attached managed picker route must
    // hit the managed resolver's privacy gate before any backend client exists.
    for route in ["openhuman", "openrouter/author/picked-model:free"] {
        let result = if route == "openhuman" {
            create_chat_model_from_string("chat", route, &Config::default(), 0.0).await
        } else {
            managed_backend::make_openhuman_backend_model_for_thread(
                "chat",
                &Config::default(),
                Some(route),
                true,
                Some("privacy-test-thread"),
                &EgressLedger::none(),
            )
            .map(|(model, _)| model)
        };
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("managed inference must be refused in LocalOnly mode"),
        };
        assert!(error.to_string().contains("Local-only privacy mode"));
    }

    // Local provider passes.
    enforce_local_only_inference("chat", "ollama:llama3")
        .expect("local provider must be permitted in LocalOnly mode");

    // Restore Standard so we don't leak LocalOnly into other serial tests.
    crate::security::live_policy::reload_privacy(PrivacyMode::Standard).expect("policy installed");
}

// ── Phase 1 (#4249): `create_chat_model` seam ──────────────────────────────
// The crate `ChatModel` factory must return the injected crate-native model
// directly; a one-shot `invoke` round-trips without a Provider adapter.
#[tokio::test]
async fn create_chat_model_uses_native_test_override() {
    use std::sync::Arc;
    use tinyagents_harness::testkit::ScriptedModel;
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::ModelRequest;

    let _guard = crate::inference::inference_test_guard_async().await;

    // The factory consults this override under cfg(test), so `create_chat_model`
    // resolves to the mock without needing configured cloud providers.
    let _override = test_provider_override::install_model(Arc::new(ScriptedModel::replies(vec![
        "echo: hi there",
    ])));
    let config = Config::default();

    let model = create_chat_model_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.3,
    )
    .await
    .expect("create_chat_model must build");
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
    use tinyinference_llm::message::Message;
    use tinyinference_llm::model::{ModelRequest, ModelResponse};

    struct TemperatureProbe {
        seen: Arc<Mutex<Vec<Option<f64>>>>,
    }

    #[async_trait]
    impl ChatModel<()> for TemperatureProbe {
        async fn invoke(
            &self,
            _state: &(),
            request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            self.seen
                .lock()
                .expect("probe lock")
                .push(request.temperature);
            Ok(ModelResponse::assistant("ok"))
        }
    }

    let _guard = crate::inference::inference_test_guard_async().await;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let _override = test_provider_override::install_model(Arc::new(TemperatureProbe {
        seen: Arc::clone(&seen),
    }));

    let config = Config::default();
    let role_model = create_chat_model_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.3,
    )
    .await
    .expect("role model");
    role_model
        .invoke(&(), ModelRequest::new(vec![Message::user("default")]))
        .await
        .expect("default-temperature invoke");

    let explicit_model = create_chat_model_from_string("chat", "openhuman", &config, 0.7)
        .await
        .expect("explicit provider model");
    explicit_model
        .invoke(
            &(),
            ModelRequest::new(vec![Message::user("explicit")]).with_temperature(0.9),
        )
        .await
        .expect("explicit-temperature invoke");

    assert_eq!(
        *seen.lock().expect("probe lock"),
        vec![Some(0.3), Some(0.9)]
    );
}

#[tokio::test]
async fn one_shot_cloud_sentinel_resolves_the_configured_byok_provider() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env = crate::config::TEST_ENV_LOCK.lock().await;
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let mut provider = openai_entry("p_openai", "openai");
    provider.default_model = Some("gpt-4o-mini".into());
    let mut config = config_with_providers_in_tempdir(&tmp, vec![provider]);
    config.primary_cloud = Some("p_openai".into());
    store_byo_key(&config, "openai", "sk-fixture").await;
    let library = crate::core::runtime::CoreContext::for_test_with_host_kind(
        crate::core::types::HostKind::Library,
        crate::core::runtime::DomainSet::full(),
        Some(tmp.path().join("library")),
    );

    crate::core::runtime::CoreContext::scope(library, async {
        for provider in ["cloud", ""] {
            let (_, model) =
                create_chat_model_from_string_with_model_id("chat", provider, &config, 0.2)
                    .await
                    .expect("sentinel resolves to the configured BYOK route");
            assert_eq!(model, "gpt-4o-mini");
        }
    })
    .await;
}

// ── Motion B (#4727): managed-backend crate-native routing ──────────────────
// `create_chat_model` must route the managed OpenHuman backend through the
// crate-native `TinyHumansModel`, whose profile advertises the capabilities
// that routing previously inferred through the provider adapter.

#[tokio::test]
async fn resolves_to_managed_backend_for_default_config_but_not_for_local() {
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

#[tokio::test]
async fn create_chat_model_routes_managed_backend_to_crate_native() {
    let _guard = crate::inference::inference_test_guard();
    install_test_managed_provider();
    let (tmp, mut config) = isolated_config();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    store_fixture_session(&config).await;
    let (_model, model_id) =
        create_chat_model_with_model_id_in(&fixture_library_dispatch(), "chat", &config, 0.7)
            .await
            .expect("managed create_chat_model must build");
    assert_eq!(model_id, "fixture/managed-default");
    crate::inference::registry::unregister_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
    );
}

#[tokio::test]
async fn create_chat_model_routes_local_runtime_to_crate_native() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.chat_provider = Some("ollama:qwen2.5".to_string());
    let (model, model_id) = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
    .await
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

#[tokio::test]
async fn explicit_local_provider_string_routes_to_crate_native_model() {
    let _guard = crate::inference::inference_test_guard();
    let config = Config::default();
    let (model, model_id) =
        create_chat_model_from_string_with_model_id("chat", "ollama:qwen2.5", &config, 0.7)
            .await
            .expect("explicit local model must build");
    assert_eq!(model_id, "qwen2.5");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("ollama")
    );
}

#[tokio::test]
async fn try_create_local_runtime_returns_none_for_managed_and_cloud() {
    let _guard = crate::inference::inference_test_guard();
    // Default config resolves to the managed backend, not a local runtime.
    assert!(super::local_runtime::try_create_local_runtime_chat_model(
        "chat",
        &Config::default(),
        &EgressLedger::none(),
    )
    .is_none());
    // A BYOK cloud slug is not a local runtime either — it falls through to the
    // `Provider` path.
    let mut cloud = Config::default();
    cloud.cloud_providers.push(openai_entry("p_oai", "openai"));
    cloud.chat_provider = Some("openai:gpt-4o-mini".to_string());
    assert!(super::local_runtime::try_create_local_runtime_chat_model(
        "chat",
        &cloud,
        &EgressLedger::none(),
    )
    .is_none());
}

#[tokio::test]
async fn create_chat_model_routes_plain_bearer_cloud_slug_to_crate_native() {
    let _guard = crate::inference::inference_test_guard();
    // DeepSeek is a built-in chat-completions-only Bearer provider: no
    // `/v1/responses` fallback and no codex-oauth, so it is wire-equivalent and
    // flips crate-native.
    let (_tmp, mut config) = isolated_config();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    config.chat_provider = Some("deepseek:deepseek-reasoner".to_string());
    store_fixture_cloud_key(&config, "deepseek").await;
    let (model, model_id) =
        create_chat_model_with_model_id_in(&fixture_library_dispatch(), "chat", &config, 0.7)
            .await
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

#[tokio::test]
async fn explicit_cloud_provider_string_routes_to_crate_native_model() {
    let _guard = crate::inference::inference_test_guard();
    let (_tmp, mut config) = isolated_config();
    config.cloud_providers.push(deepseek_entry("p_ds"));
    store_fixture_cloud_key(&config, "deepseek").await;
    let model_and_id = crate::core::runtime::CoreContext::scope(
        fixture_library_dispatch()
            .resolved()
            .expect("library context"),
        async {
            create_chat_model_from_string_with_model_id(
                "chat",
                "deepseek:deepseek-reasoner",
                &config,
                0.7,
            )
            .await
        },
    )
    .await;
    let (model, model_id) = model_and_id.expect("explicit cloud model must build");
    assert_eq!(model_id, "deepseek-reasoner");
    assert_eq!(
        model
            .profile()
            .and_then(|profile| profile.provider.as_deref()),
        Some("deepseek")
    );
}

#[tokio::test]
async fn create_chat_model_routes_anthropic_auth_cloud_slug_to_crate_native() {
    let _guard = crate::inference::inference_test_guard();
    // Anthropic-auth cloud slugs are always wire-equivalent (their endpoints have
    // no `/v1/responses`, so the host's dormant fallback is behavior-neutral).
    let (_tmp, mut config) = isolated_config();
    config
        .cloud_providers
        .push(anthropic_entry("p_anth", "anthropic"));
    config.chat_provider = Some("anthropic:claude-sonnet-4-6".to_string());
    store_fixture_cloud_key(&config, "anthropic").await;
    let (model, model_id) =
        create_chat_model_with_model_id_in(&fixture_library_dispatch(), "chat", &config, 0.7)
            .await
            .expect("anthropic cloud create_chat_model must build");
    assert_eq!(model_id, "claude-sonnet-4-6");
    assert_eq!(
        model.profile().and_then(|p| p.provider.as_deref()),
        Some("anthropic")
    );
    // The native Messages API adapter, not the OpenAI-compat flavour: its
    // cache identity is `anthropic:<base>:<model>`, while the compat client
    // would report the provider slug with an OpenAI-shaped identity. Only the
    // native adapter places `cache_control` breakpoints — Anthropic's compat
    // endpoint documents prompt caching as unsupported.
    let identity = model.cache_identity().expect("anthropic identifies itself");
    assert!(
        identity.starts_with("anthropic:https://api.anthropic.com/v1:claude-sonnet-4-6"),
        "expected the native Messages adapter, got identity {identity}"
    );
    assert!(model
        .profile()
        .is_some_and(|p| p.tool_calling && p.streaming));
}

/// Text mode (prompt-guided tools) is only implemented on the Chat Completions
/// adapter, so an Anthropic slug asked for `native_tools = false` keeps the
/// compat client rather than a native adapter that would ignore the request.
#[tokio::test]
async fn anthropic_auth_cloud_slug_in_text_mode_keeps_the_compat_client() {
    let _guard = crate::inference::inference_test_guard();
    let (_tmp, mut config) = isolated_config();
    config
        .cloud_providers
        .push(anthropic_entry("p_anth", "anthropic"));
    store_fixture_cloud_key(&config, "anthropic").await;
    let (model, model_id) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        "chat",
        "anthropic:claude-sonnet-4-6",
        &config,
        false,
        &EgressLedger::none(),
    )
    .await
    .expect("anthropic slug resolves")
    .expect("build");
    assert_eq!(model_id, "claude-sonnet-4-6");
    let identity = model.cache_identity().unwrap_or_default();
    assert!(
        !identity.starts_with("anthropic:https://"),
        "text mode must not route to the native adapter, got {identity}"
    );
    assert!(model.profile().is_some_and(|p| !p.tool_calling));
}

#[tokio::test]
async fn configured_openhuman_jwt_slug_routes_to_managed_chat_model() {
    let _guard = crate::inference::inference_test_guard();
    install_test_managed_provider();
    let (_tmp, mut config) = isolated_config();
    config
        .cloud_providers
        .push(fixture_managed_entry("p_fixture"));
    config.chat_provider = Some("fixture-managed:hint:reasoning".to_string());

    let (_model, model_id) =
        try_create_cloud_slug_chat_model("chat", &config, &EgressLedger::none())
            .await
            .expect("registered host policy should recognize this route")
            .expect("registered model should build");

    assert_eq!(model_id, "fixture/managed-default");

    // A concrete catalog id is forwarded verbatim.
    config.chat_provider = Some("fixture-managed:vendor/pinned".to_string());
    let (_, pinned_id) = try_create_cloud_slug_chat_model("chat", &config, &EgressLedger::none())
        .await
        .expect("registered host policy should recognize this route")
        .expect("registered model should build");
    assert_eq!(pinned_id, "vendor/pinned");
    crate::inference::registry::unregister_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
    );
}

#[tokio::test]
async fn openhuman_jwt_slug_discloses_pinned_model() {
    use crate::core::events::DomainEvent;
    use crate::security::egress::{EgressDescriptor, EgressReason};
    use std::time::Duration;

    let _guard = crate::inference::inference_test_guard_async().await;
    install_test_managed_provider();
    crate::core::bus::init().await.expect("bus init");
    let mut rx = crate::core::bus::BUS.get().unwrap().receiver();

    let marker = "egress-jwt-pinned-marker-v1";
    let (_tmp, mut config) = isolated_config();
    config
        .cloud_providers
        .push(fixture_managed_entry("p_fixture"));
    let provider = format!("fixture-managed:{marker}");
    let _ = try_create_cloud_slug_chat_model_from_string(
        "chat",
        &provider,
        &config,
        &EgressLedger::none(),
    )
    .await
    .expect("registered host policy should recognize this route")
    .expect("registered model should build");

    let sentinel = "egress-jwt-pinned-sentinel-end";
    crate::core::bus::BUS.publish(DomainEvent::ExternalTransferPending {
        descriptor: EgressDescriptor::network_fetch(sentinel),
        thread_id: None,
        client_id: None,
        request_id: None,
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
    crate::inference::registry::unregister_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
    );
}

#[tokio::test]
async fn openhuman_jwt_slug_preserves_forced_text_mode() {
    let _guard = crate::inference::inference_test_guard();
    let native_modes = install_test_managed_provider();
    let (_tmp, mut config) = isolated_config();
    config
        .cloud_providers
        .push(fixture_managed_entry("p_fixture"));

    let (_model, _) = try_create_cloud_slug_chat_model_from_string_with_native_tools(
        "chat",
        "fixture-managed:hint:reasoning",
        &config,
        false,
        &EgressLedger::none(),
    )
    .await
    .expect("registered host policy should recognize this route")
    .expect("registered model should build");

    assert_eq!(native_modes.lock().unwrap().as_slice(), [false]);
    crate::inference::registry::unregister_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
    );
}

#[tokio::test]
async fn openhuman_jwt_slug_without_model_preserves_managed_role_tier() {
    let _guard = crate::inference::inference_test_guard();
    install_test_managed_provider();
    let (_tmp, mut config) = isolated_config();
    config
        .cloud_providers
        .push(fixture_managed_entry("p_fixture"));

    let (_model, model_id) = try_create_cloud_slug_chat_model_from_string(
        "summarization",
        "fixture-managed:",
        &config,
        &EgressLedger::none(),
    )
    .await
    .expect("registered host policy should recognize this route")
    .expect("registered model should build");

    assert_eq!(model_id, "fixture/managed-default");
    crate::inference::registry::unregister_chat_model(
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
    );
}

#[path = "factory_crate_native_cloud_tests.rs"]
mod cloud_tests;

#[path = "factory_crate_native_defaults_tests.rs"]
mod default_temperature_tests;

#[path = "factory_crate_native_egress_tests.rs"]
mod egress_tests;
