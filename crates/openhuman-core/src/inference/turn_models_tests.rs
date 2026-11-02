use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::host::{ModelResolveRequest, ModelResolver};
use tinyinference_llm::message::ContentBlock;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

async fn managed_fixture_config(tmp: &tempfile::TempDir) -> crate::config::Config {
    let config = crate::config::Config {
        config_path: tmp.path().join("config.toml"),
        workspace_dir: tmp.path().join("workspace"),
        ..Default::default()
    };
    crate::credentials::AuthService::from_config(&config)
        .store_provider_token(
            crate::credentials::APP_SESSION_PROVIDER,
            "default",
            "fixture-session-token",
            Default::default(),
            true,
        )
        .await
        .expect("store isolated fixture session");
    config
}

use super::TurnModels;
use crate::agent::tinyagents::TurnModelSource;
use tinyagents_harness::host::PrebuiltModelResolver as TurnModelResolver;

fn scoped_model_entry(
    name: &'static str,
    supports_vision: bool,
) -> tinyinference_llm::providers::overrides::ChatModelEntry<
    crate::inference::registry::ProviderContext,
> {
    use tinyinference_llm::providers::overrides::ProviderModelMetadata;

    tinyinference_llm::providers::overrides::ChatModelEntry::<
        crate::inference::registry::ProviderContext,
    > {
        build: Arc::new(move |_| Ok((Arc::new(NamedModel(name)), name.to_string()))),
        probe: None,
        models: Some(ProviderModelMetadata {
            default_model: Arc::new(move || Some(name.to_string())),
            role_for_model: Arc::new(move |model| {
                if model == "fixture-role" {
                    Some(
                        if supports_vision {
                            "reasoning"
                        } else {
                            "coding"
                        }
                        .to_string(),
                    )
                } else {
                    model.strip_prefix("hint:").map(str::to_string)
                }
            }),
            is_alias: Arc::new(|model| model.starts_with("hint:")),
            is_raw_model: Arc::new(|model| !model.starts_with("hint:")),
            supports_vision: Arc::new(move |_| supports_vision),
            resolve_model: Arc::new(move |_, _, model| match model {
                Some(model) if !model.starts_with("hint:") => Some(model.to_string()),
                Some(_) | None => Some(name.to_string()),
            }),
            context_window: Arc::new(move |_| Some(if supports_vision { 131_072 } else { 32_768 })),
            default_pricing: Arc::new(|| None),
            without_reasoning: Arc::new(|request| request),
        }),
    }
}

#[tokio::test]
async fn explicit_turn_context_uses_its_model_and_capabilities_over_ambient_context() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;

    let provider = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    let context_a_extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &context_a_extensions,
        provider,
        scoped_model_entry("context-a-model", true),
    );
    let context_a = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        context_a_extensions,
    );

    let context_b_extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &context_b_extensions,
        provider,
        scoped_model_entry("context-b-model", false),
    );
    let context_b = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        context_b_extensions,
    );

    let dispatch = DispatchCtx::new(Arc::clone(&context_a));
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = Arc::new(managed_fixture_config(&tmp).await);
    CoreContext::scope(context_b, async {
        let source = TurnModelSource::new_crate_native(&dispatch, "chat", config);
        let models = source
            .build("hint:chat", 0.0, None, None)
            .await
            .expect("explicit context A has a registered managed model");

        assert_eq!(name_of(&models.primary).await, "context-a-model");
        assert_eq!(name_of(&models.summarizer).await, "context-a-model");
        let standalone = source
            .build_summarizer("hint:summarization", 0.0, None)
            .await
            .expect("standalone summarizer uses explicit context A");
        assert_eq!(name_of(&standalone).await, "context-a-model");
        assert!(models
            .primary
            .profile()
            .is_some_and(|profile| profile.modalities.image_in));
        let extensions = context_a.extensions();
        assert_eq!(
            crate::inference::registry::provider_model_role_for(
                Some(extensions),
                provider,
                "fixture-role"
            )
            .as_deref(),
            Some("reasoning")
        );
        assert_eq!(
            crate::inference::registry::provider_model_context_window_for(
                Some(extensions),
                provider,
                "context-a-model"
            ),
            Some(131_072)
        );
        assert_eq!(
            source.effective_context_window("context-a-model").await,
            Some(131_072)
        );
    })
    .await;
}

#[tokio::test]
async fn explicit_turn_context_without_provider_does_not_borrow_ambient_provider() {
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use crate::core::slots::Extensions;

    let context_a = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        Extensions::new(),
    );
    let context_b_extensions = Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &context_b_extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        scoped_model_entry("context-b-model", false),
    );
    let context_b = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        context_b_extensions,
    );

    let dispatch = DispatchCtx::new(context_a);
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = Arc::new(managed_fixture_config(&tmp).await);
    CoreContext::scope(context_b, async {
        let source = TurnModelSource::new_crate_native(&dispatch, "chat", config);
        let error = source
            .build("hint:chat", 0.0, None, None)
            .await
            .err()
            .expect("an explicit context without a provider must fail closed");
        assert!(error.to_string().contains("BACKEND_UNAVAILABLE"));
    })
    .await;
}

#[tokio::test]
async fn role_pins_apply_when_the_primary_uses_config_routing() {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let tmp = tempfile::tempdir().expect("isolated workspace");
    let config = managed_fixture_config(&tmp).await;
    let extensions = crate::core::slots::Extensions::new();
    crate::inference::registry::register_chat_model_in(
        &extensions,
        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        tinyinference_llm::providers::overrides::ChatModelEntry::<
            crate::inference::registry::ProviderContext,
        > {
            build: Arc::new(|_| {
                Ok((
                    Arc::new(NamedModel("managed fixture")),
                    "fixture/managed-default".into(),
                ))
            }),
            probe: None,
            models: Some(
                tinyinference_llm::providers::overrides::ProviderModelMetadata {
                    default_model: Arc::new(|| Some("fixture/managed-default".into())),
                    role_for_model: Arc::new(|model| {
                        model
                            .strip_prefix("hint:")
                            .filter(|role| matches!(*role, "chat" | "coding" | "summarization"))
                            .map(str::to_string)
                    }),
                    is_alias: Arc::new(|model| model.starts_with("hint:")),
                    is_raw_model: Arc::new(|model| !model.starts_with("hint:")),
                    supports_vision: Arc::new(|_| false),
                    resolve_model: Arc::new(|_, config, model| {
                        model
                            .filter(|model| !model.starts_with("hint:"))
                            .map(str::to_string)
                            .or_else(|| config.config.default_model.clone())
                            .or_else(|| Some("fixture/managed-default".into()))
                    }),
                    context_window: Arc::new(|_| None),
                    default_pricing: Arc::new(|| None),
                    without_reasoning: Arc::new(|request| request),
                },
            ),
        },
    );
    let mut overrides = HostOverrides::default();
    overrides
        .role_models
        .insert("coding".into(), Arc::new(NamedModel("pinned coding")));
    overrides.role_models.insert(
        "summarization".into(),
        Arc::new(NamedModel("pinned summary")),
    );
    let root = CoreContext::for_test_with_host_kind_and_extensions(
        crate::core::types::HostKind::Library,
        DomainSet::full(),
        None,
        extensions,
    );
    let context = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), Default::default())
            .host_overrides(Arc::new(overrides)),
    );
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(context);

        let models = TurnModelSource::new_crate_native(&scope_ctx, "chat", Arc::new(config))
            .build("hint:chat", 0.0, None, None)
            .await
            .expect("config-routed models");
        let coding = models
            .routes
            .iter()
            .find(|(tier, _)| tier == "hint:coding")
            .expect("pinned role route");
        assert_eq!(name_of(&coding.1).await, "pinned coding");
        assert_eq!(name_of(&models.summarizer).await, "pinned summary");
    };
}

/// A stub that answers with its own name so a test can tell which model the
/// resolver handed back.
struct NamedModel(&'static str);

#[async_trait]
impl ChatModel<()> for NamedModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference_llm::Result<ModelResponse> {
        Ok(ModelResponse::assistant(self.0))
    }
}

async fn name_of(model: &Arc<dyn ChatModel<()>>) -> String {
    let response = model
        .invoke(&(), ModelRequest::default())
        .await
        .expect("stub model never fails");
    response
        .message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

fn resolver() -> TurnModelResolver<()> {
    let primary: Arc<dyn ChatModel<()>> =
        Arc::new(NamedModel("openrouter/deepseek/deepseek-v4.1-flash"));
    let mut routes: HashMap<String, Arc<dyn ChatModel<()>>> = HashMap::new();
    routes.insert(
        "hint:coding".to_string(),
        Arc::new(NamedModel("hint:coding")),
    );
    routes.insert("hint:burst".to_string(), Arc::new(NamedModel("hint:burst")));
    TurnModelResolver::new(primary, routes)
}

/// Regression for the orchestrator's `hint = "coding"` overriding the user's
/// UI model pick: the turn lead must get the selected primary even when its
/// definition pin names a built tier route.
#[tokio::test]
async fn lead_keeps_selected_primary_over_definition_pin() {
    let request = ModelResolveRequest::new("orchestrator")
        .as_team_lead()
        .with_model_pin("hint:coding");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(
        name_of(&model).await,
        "openrouter/deepseek/deepseek-v4.1-flash"
    );
}

#[tokio::test]
async fn lead_without_pin_gets_primary() {
    let request = ModelResolveRequest::new("orchestrator").as_team_lead();
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(
        name_of(&model).await,
        "openrouter/deepseek/deepseek-v4.1-flash"
    );
}

#[tokio::test]
async fn subagent_pin_resolves_to_its_tier_route() {
    let request = ModelResolveRequest::new("integrations_agent").with_model_pin("hint:burst");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(name_of(&model).await, "hint:burst");
}

#[tokio::test]
async fn subagent_pin_without_route_falls_back_to_primary() {
    let request = ModelResolveRequest::new("worker").with_model_pin("hint:vision");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(
        name_of(&model).await,
        "openrouter/deepseek/deepseek-v4.1-flash"
    );
}

#[tokio::test]
async fn subagent_typed_hint_resolves_to_its_tier_route() {
    let request = ModelResolveRequest::new("integrations_agent").with_model_hint("burst");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(name_of(&model).await, "hint:burst");
}

#[tokio::test]
async fn lead_keeps_selected_primary_over_typed_hint() {
    let request = ModelResolveRequest::new("orchestrator")
        .as_team_lead()
        .with_model_hint("coding");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(
        name_of(&model).await,
        "openrouter/deepseek/deepseek-v4.1-flash"
    );
}

#[tokio::test]
async fn explicit_pin_takes_precedence_over_typed_hint() {
    let request = ModelResolveRequest::new("worker")
        .with_model_pin("hint:coding")
        .with_model_hint("burst");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(name_of(&model).await, "hint:coding");
}

#[tokio::test]
async fn unconfigured_typed_hint_falls_back_to_primary() {
    let request = ModelResolveRequest::new("worker").with_model_hint("vision");
    let model = resolver().resolve(&request).await.expect("resolves");
    assert_eq!(
        name_of(&model).await,
        "openrouter/deepseek/deepseek-v4.1-flash"
    );
}

/// Streams the next scripted terminal item on each call.
struct ScriptedTerminalModel(std::sync::Mutex<Vec<tinyinference_llm::model::ModelStreamItem>>);

#[async_trait::async_trait]
impl ChatModel<()> for ScriptedTerminalModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelResponse> {
        unreachable!("these tests stream")
    }

    async fn stream(
        &self,
        _state: &(),
        _request: tinyinference_llm::model::ModelRequest,
    ) -> tinyinference_llm::Result<tinyinference_llm::model::ModelStream> {
        let item = self.0.lock().unwrap().remove(0);
        Ok(tinyinference_llm::model::ModelStream::new(Box::pin(
            futures::stream::iter(vec![item]),
        )))
    }
}

fn provider_failure(message: &str) -> tinyinference_llm::model::ModelStreamItem {
    tinyinference_llm::model::ModelStreamItem::ProviderFailed(
        tinyinference_llm::model::ProviderError {
            provider: "OpenHuman".to_string(),
            status: Some(400),
            message: message.to_string(),
            ..Default::default()
        },
    )
}

fn completed() -> tinyinference_llm::model::ModelStreamItem {
    tinyinference_llm::model::ModelStreamItem::Completed(
        serde_json::from_value(serde_json::json!({
            "message": {"content": [], "tool_calls": []}
        }))
        .expect("minimal response"),
    )
}

async fn drain(model: &Arc<dyn ChatModel<()>>) {
    use futures::StreamExt;
    let mut stream = model
        .stream(&(), tinyinference_llm::model::ModelRequest::default())
        .await
        .unwrap();
    while stream.next().await.is_some() {}
}

fn slot_text(models: &TurnModels) -> Option<String> {
    models
        .error_slot
        .lock()
        .unwrap()
        .as_ref()
        .map(|error| error.to_string())
}

/// #6724: sub-agents resolve through the lead's resolver, possibly in
/// parallel. A child's provider failure must not become the lead's run error.
#[tokio::test]
async fn a_sub_agents_provider_failure_is_not_recorded_as_the_leads() {
    let model: Arc<dyn ChatModel<()>> =
        Arc::new(ScriptedTerminalModel(std::sync::Mutex::new(vec![
            provider_failure("CHILD_FAILURE"),
        ])));
    let models = TurnModelSource::from_model(model)
        .build("m", 0.0, None, None)
        .await
        .expect("turn models");
    let resolver = models.resolver();
    let child = resolver
        .resolve(&ModelResolveRequest::new("researcher"))
        .await
        .unwrap();
    drain(&child).await;
    assert_eq!(
        slot_text(&models),
        None,
        "a child failure leaked into the lead's slot"
    );
}

/// #6724: a child's attempt (here a success) must not clear the failure the
/// lead already recorded.
#[tokio::test]
async fn a_sub_agents_success_does_not_clear_the_leads_recorded_failure() {
    let model: Arc<dyn ChatModel<()>> =
        Arc::new(ScriptedTerminalModel(std::sync::Mutex::new(vec![
            provider_failure("LEAD_FAILURE"),
            completed(),
        ])));
    let models = TurnModelSource::from_model(model)
        .build("m", 0.0, None, None)
        .await
        .expect("turn models");
    let resolver = models.resolver();
    let lead = resolver
        .resolve(&ModelResolveRequest::new("orchestrator").as_team_lead())
        .await
        .unwrap();
    drain(&lead).await;
    assert!(slot_text(&models).is_some_and(|text| text.contains("LEAD_FAILURE")));

    let child = resolver
        .resolve(&ModelResolveRequest::new("researcher"))
        .await
        .unwrap();
    drain(&child).await;
    assert!(
        slot_text(&models).is_some_and(|text| text.contains("LEAD_FAILURE")),
        "a child's attempt cleared the lead's recorded failure"
    );
}

/// #6724: a child that resolves a real tier route (not the primary fallback)
/// and fails must leave the lead's recorded failure untouched.
#[tokio::test]
async fn a_sub_agents_route_failure_leaves_the_leads_slot_untouched() {
    let primary: Arc<dyn ChatModel<()>> =
        Arc::new(ScriptedTerminalModel(std::sync::Mutex::new(vec![
            provider_failure("LEAD_FAILURE"),
            // Spare item: a child that wrongly got the primary would consume it
            // silently, so only the route assertion can catch that.
            completed(),
        ])));
    let route = Arc::new(ScriptedTerminalModel(std::sync::Mutex::new(vec![
        provider_failure("CHILD_ROUTE_FAILURE"),
    ])));
    let models = TurnModelSource::from_model(primary)
        .build("m", 0.0, None, None)
        .await
        .expect("turn models")
        .with_test_route("hint:burst", route.clone());
    let resolver = models.resolver();

    let lead = resolver
        .resolve(&ModelResolveRequest::new("orchestrator").as_team_lead())
        .await
        .unwrap();
    drain(&lead).await;

    let child = resolver
        .resolve(&ModelResolveRequest::new("integrations_agent").with_model_pin("hint:burst"))
        .await
        .unwrap();
    drain(&child).await;
    // Self-proving fixture: the child's call was served by the route, not by a
    // fallback to the primary.
    assert!(
        route.0.lock().unwrap().is_empty(),
        "the child did not resolve the route"
    );

    let recorded = slot_text(&models).expect("the lead's failure is still recorded");
    assert!(recorded.contains("LEAD_FAILURE"), "{recorded}");
    assert!(
        !recorded.contains("CHILD_ROUTE_FAILURE"),
        "a child's route failure replaced the lead's: {recorded}"
    );
}

/// #6724: a child's successful call on a real route must not clear the failure
/// the lead already recorded.
#[tokio::test]
async fn a_sub_agents_route_success_does_not_clear_the_leads_slot() {
    let primary: Arc<dyn ChatModel<()>> =
        Arc::new(ScriptedTerminalModel(std::sync::Mutex::new(vec![
            provider_failure("LEAD_FAILURE"),
            // Spare item: a child that wrongly got the primary would consume it
            // silently, so only the route assertion can catch that.
            completed(),
        ])));
    let route = Arc::new(ScriptedTerminalModel(std::sync::Mutex::new(vec![
        completed(),
    ])));
    let models = TurnModelSource::from_model(primary)
        .build("m", 0.0, None, None)
        .await
        .expect("turn models")
        .with_test_route("hint:burst", route.clone());
    let resolver = models.resolver();

    let lead = resolver
        .resolve(&ModelResolveRequest::new("orchestrator").as_team_lead())
        .await
        .unwrap();
    drain(&lead).await;
    let child = resolver
        .resolve(&ModelResolveRequest::new("integrations_agent").with_model_pin("hint:burst"))
        .await
        .unwrap();
    drain(&child).await;
    // Self-proving fixture: the child's call was served by the route, not by a
    // fallback to the primary.
    assert!(
        route.0.lock().unwrap().is_empty(),
        "the child did not resolve the route"
    );

    assert!(
        slot_text(&models).is_some_and(|text| text.contains("LEAD_FAILURE")),
        "a child's route success cleared the lead's recorded failure"
    );
}

#[path = "turn_models_native_tests.rs"]
mod native_tests;

#[path = "turn_models_summarizer_tests.rs"]
mod summarizer_tests;

#[path = "turn_models_budget_media_tests.rs"]
mod budget_media_tests;
