use super::*;

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut config = Config::default();
    config.workspace_dir = tmp.path().join("workspace");
    config.config_path = tmp.path().join("config.toml");
    config
}

// ── `model_override` under a pinned provider route ────────────────────────

/// Endpoint overrides that outrank `local_ai.base_url`; unset so the
/// fixture's mock endpoint is the one the local runtime resolves.
const LOCAL_ENDPOINT_ENV: &[&str] = &["OPENHUMAN_LOCAL_INFERENCE_URL", "LOCAL_OPENAI_URL"];

/// The model the bootstrap pinned every agent-turn role to, the way
/// `bin/spine` does: `default_model = M` and each role on `local-openai:M`.
const PINNED_MODEL: &str = "pinned-reasoning-model";

fn pinned_local_route_config(
    tmp: &tempfile::TempDir,
    server: &wiremock::MockServer,
) -> crate::config::Config {
    let mut config = test_config(tmp);
    config.local_ai.base_url = Some(format!("{}/v1", server.uri()));
    config.default_model = Some(PINNED_MODEL.to_string());
    let route = format!("local-openai:{PINNED_MODEL}");
    config.chat_provider = Some(route.clone());
    config.reasoning_provider = Some(route.clone());
    config.agentic_provider = Some(route.clone());
    config.coding_provider = Some(route.clone());
    config.vision_provider = Some(route.clone());
    config.memory_provider = Some(route);
    config
}

async fn mock_local_endpoint() -> wiremock::MockServer {
    use wiremock::matchers::{method, path};
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "pinned-route-completion",
                "model": "whatever-the-endpoint-says",
                "choices": [{
                    "index": 0,
                    "message": {"role": "assistant", "content": "ok"},
                    "finish_reason": "stop"
                }],
                "usage": {"prompt_tokens": 2, "completion_tokens": 1, "total_tokens": 3}
            })),
        )
        .mount(&server)
        .await;
    server
}

/// Builds the turn's model the way the session builder does (the chat role's
/// model id, as `resolve_session_model` resolves it, then the crate-native
/// turn model set), sends one request through it, and returns the session's
/// resolved model and the request body the endpoint received.
async fn send_one_turn_request(
    config: &crate::config::Config,
    server: &wiremock::MockServer,
    reasoning: Option<tinyinference_llm::model::ReasoningConfig>,
) -> (String, serde_json::Value) {
    let ctx = crate::core::runtime::DispatchCtx::unbound();
    let role = "chat";
    let (_, model_name) =
        crate::inference::provider::create_chat_model_with_model_id_in(&ctx, role, config, 0.0)
            .await
            .expect("session model resolves");
    let models = crate::agent::tinyagents::TurnModelSource::new_crate_native(
        &ctx,
        role,
        std::sync::Arc::new(config.clone()),
    )
    .build(&model_name, 0.0, None, None)
    .await
    .expect("turn model set builds");
    let mut request = tinyinference_llm::model::ModelRequest::new(vec![
        tinyinference_llm::message::Message::user("judge this"),
    ]);
    if let Some(reasoning) = reasoning {
        request = request.with_reasoning(reasoning);
    }
    models
        .primary
        .invoke(&(), request)
        .await
        .expect("the endpoint answers");
    let received = server.received_requests().await.expect("recorded requests");
    let body: serde_json::Value = received
        .last()
        .expect("the endpoint was called")
        .body_json()
        .expect("json request body");
    (model_name, body)
}

/// The judge's bug: `bin/spine` pins every role to `local-openai:<M>`, and a
/// turn's `model_override` only replaced `default_model`, which the pinned
/// route outranks, so the endpoint was still asked for `<M>`.
#[tokio::test]
async fn model_override_reaches_the_endpoint_under_a_pinned_local_route() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env =
        crate::config::test_env::EnvVarGuard::locked_unset_many_async(LOCAL_ENDPOINT_ENV).await;
    let server = mock_local_endpoint().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = pinned_local_route_config(&tmp, &server);

    apply_turn_overrides(&mut config, Some("fast-judge-model".into()), None, None);

    let (resolved, body) = send_one_turn_request(&config, &server, None).await;
    assert_eq!(body["model"], "fast-judge-model", "wire model: {body}");
    assert_eq!(resolved, "fast-judge-model");
    // The turn's other agent roles follow it; background workloads keep
    // the pinned model (an embeddings or memory model is not swappable).
    for role in ["chat", "reasoning", "agentic", "coding"] {
        assert_eq!(
            crate::inference::provider::provider_for_role(role, &config),
            "local-openai:fast-judge-model",
            "role {role}"
        );
    }
    assert_eq!(
        config.memory_provider.as_deref(),
        Some("local-openai:pinned-reasoning-model")
    );
    assert_eq!(
        config.vision_provider.as_deref(),
        Some("local-openai:pinned-reasoning-model")
    );
}

/// No override: the pinned route's model is what the endpoint is asked for.
#[tokio::test]
async fn turn_without_model_override_keeps_the_pinned_route_model() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env =
        crate::config::test_env::EnvVarGuard::locked_unset_many_async(LOCAL_ENDPOINT_ENV).await;
    let server = mock_local_endpoint().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = pinned_local_route_config(&tmp, &server);

    apply_turn_overrides(&mut config, None, None, None);
    let (resolved, body) = send_one_turn_request(&config, &server, None).await;
    assert_eq!(body["model"], PINNED_MODEL, "wire model: {body}");
    assert_eq!(resolved, PINNED_MODEL);

    // A blank override is no override.
    apply_turn_overrides(&mut config, Some("  ".into()), None, None);
    let (_, body) = send_one_turn_request(&config, &server, None).await;
    assert_eq!(body["model"], PINNED_MODEL, "wire model: {body}");
}

/// A `hint:<role>` override picks a workload, not a model: the pinned routes
/// stay as they are rather than becoming `local-openai:hint:<role>`.
#[test]
fn hint_model_override_selects_a_role_without_rewriting_the_pinned_routes() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(&tmp);
    config.chat_provider = Some("local-openai:pinned".into());
    config.reasoning_provider = Some("local-openai:pinned".into());

    apply_turn_overrides(&mut config, Some("hint:reasoning".into()), None, None);

    assert_eq!(config.default_model.as_deref(), Some("hint:reasoning"));
    assert_eq!(config.chat_provider.as_deref(), Some("local-openai:pinned"));
    assert_eq!(
        config.reasoning_provider.as_deref(),
        Some("local-openai:pinned")
    );
}

/// An override that is itself a provider route (`<slug>:<model>`) replaces
/// the pinned route; a bare model id keeps the pinned provider (and its
/// `@temperature`) and swaps only the model.
#[test]
fn provider_route_model_override_replaces_the_pinned_route() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = test_config(&tmp);
    config.chat_provider = Some("local-openai:pinned@0.2".into());

    let mut bare = config.clone();
    apply_turn_overrides(&mut bare, Some("other-model".into()), None, None);
    assert_eq!(
        bare.chat_provider.as_deref(),
        Some("local-openai:other-model@0.2")
    );

    apply_turn_overrides(&mut config, Some("ollama:llama3.1:8b".into()), None, None);
    assert_eq!(config.chat_provider.as_deref(), Some("ollama:llama3.1:8b"));
}

/// `inference_reasoning_effort` without `inference_url` used to ride an
/// ephemeral route that `from_params` never built, so it was dropped. It now
/// sets the turn model's effort, which the turn sends on the wire.
#[tokio::test]
async fn reasoning_effort_reaches_the_endpoint_without_an_inference_url() {
    let _guard = crate::inference::inference_test_guard_async().await;
    let _env =
        crate::config::test_env::EnvVarGuard::locked_unset_many_async(LOCAL_ENDPOINT_ENV).await;
    let server = mock_local_endpoint().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = pinned_local_route_config(&tmp, &server);
    // A level configured for the turn's model must not outrank the call's.
    config
        .runtime
        .reasoning_effort_by_model
        .insert("fast-judge-model".into(), "high".into());

    apply_turn_reasoning_effort(&mut config, Some("fast-judge-model"), Some("low".into()));
    apply_turn_overrides(&mut config, Some("fast-judge-model".into()), None, None);

    let reasoning = crate::inference::reasoning::reasoning_for_config(&config)
        .expect("the call's effort is the turn's");
    assert_eq!(
        reasoning.effort,
        Some(tinyinference_llm::model::ReasoningEffort::Low)
    );
    let (_, body) = send_one_turn_request(&config, &server, Some(reasoning)).await;
    assert_eq!(body["reasoning_effort"], "low", "wire body: {body}");
    assert_eq!(body["model"], "fast-judge-model");
}
