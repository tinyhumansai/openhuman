use super::*;
use crate::security::egress::EgressLedger;

use crate::inference::provider::factory::cloud_slug::try_create_cloud_slug_chat_model_from_string_with_native_tools;

#[test]
fn custom_ca_bundle_builds_a_provider_scoped_chat_client() {
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.cloud_providers.push(openai_entry("p_team", "team"));
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
    .expect("configured provider")
    .expect("custom CA client should build");
    assert_eq!(model_id, "private-model");
    assert_eq!(
        model.profile().and_then(|profile| profile.provider.clone()),
        Some("team".into())
    );
}
#[test]
fn enforce_local_only_inference_errors_on_external_when_local_only() {
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

    // Both the default factory and a thread-attached managed picker route must
    // hit the managed resolver's privacy gate before any backend client exists.
    for route in ["openhuman", "openrouter/author/picked-model:free"] {
        let result = if route == "openhuman" {
            create_chat_model_from_string("chat", route, &Config::default(), 0.0)
        } else {
            managed_backend::make_openhuman_backend_model_for_thread(
                "chat",
                &Config::default(),
                route,
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
    .expect("role model");
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

    let turn_model = create_turn_chat_model(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        "hint:chat",
        0.2,
    )
    .expect("turn model");
    turn_model
        .invoke(&(), ModelRequest::new(vec![Message::user("turn default")]))
        .await
        .expect("turn default-temperature invoke");

    let explicit_turn_model = create_turn_chat_model_from_string(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        "openhuman",
        &config,
        "hint:chat",
        0.4,
    )
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

#[test]
fn default_temperature_wrapper_preserves_physical_attempt_tracking() {
    use async_trait::async_trait;
    use std::sync::Arc;
    use tinyinference_llm::model::{ModelRequest, ModelResponse};

    struct TrackedModel;

    #[async_trait]
    impl ChatModel<()> for TrackedModel {
        fn tracks_physical_attempts(&self) -> bool {
            true
        }

        fn has_pre_invocation_admission_gate(&self) -> bool {
            true
        }

        async fn invoke(
            &self,
            _state: &(),
            _request: ModelRequest,
        ) -> tinyinference_llm::Result<ModelResponse> {
            Ok(ModelResponse::assistant("ok"))
        }
    }

    let _guard = crate::inference::inference_test_guard();
    let _override = test_provider_override::install_model(Arc::new(TrackedModel));
    let wrapped = create_chat_model_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &Config::default(),
        0.3,
    )
    .expect("test override model");

    assert!(wrapped.tracks_physical_attempts());
    assert!(wrapped.has_pre_invocation_admission_gate());
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
    let _guard = crate::inference::inference_test_guard();
    // No test-provider override installed → the managed short-circuit engages.
    let config = Config::default();
    let (model, _model_id) = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
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
    let _guard = crate::inference::inference_test_guard();
    let mut config = Config::default();
    config.chat_provider = Some("ollama:qwen2.5".to_string());
    let (model, model_id) = create_chat_model_with_model_id_in(
        &crate::core::runtime::DispatchCtx::ambient(),
        "chat",
        &config,
        0.7,
    )
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
    let _guard = crate::inference::inference_test_guard();
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
    let _guard = crate::inference::inference_test_guard();
    // Default config resolves to the managed backend, not a local runtime.
    assert!(
        try_create_local_runtime_chat_model("chat", &Config::default(), &EgressLedger::none())
            .is_none()
    );
    // A BYOK cloud slug is not a local runtime either — it falls through to the
    // `Provider` path.
    let mut cloud = Config::default();
    cloud.cloud_providers.push(openai_entry("p_oai", "openai"));
    cloud.chat_provider = Some("openai:gpt-4o-mini".to_string());
    assert!(try_create_local_runtime_chat_model("chat", &cloud, &EgressLedger::none()).is_none());
}

#[test]
include!("factory_crate_native_cloud_tests.rs");
