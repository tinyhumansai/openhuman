use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use tinyagents_harness::host::{ModelResolveRequest, ModelResolver};
use tinyinference_llm::message::ContentBlock;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse};

use super::{TurnModelResolver, TurnModels};
use crate::agent::tinyagents::TurnModelSource;

#[tokio::test]
async fn role_pins_apply_when_the_primary_uses_config_routing() {
    use crate::agent::host_overrides::HostOverrides;
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let config = crate::config::Config::default();
    let mut overrides = HostOverrides::default();
    overrides
        .role_models
        .insert("coding".into(), Arc::new(NamedModel("pinned coding")));
    overrides.role_models.insert(
        "summarization".into(),
        Arc::new(NamedModel("pinned summary")),
    );
    let root = CoreContext::for_test(DomainSet::full(), None);
    let context = root.derive_with(
        ContextOverlay::new(config.clone(), DomainSet::full(), Default::default())
            .host_overrides(Arc::new(overrides)),
    );
    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(context);

        let models = TurnModelSource::new_crate_native(&scope_ctx, "chat", Arc::new(config))
            .build("hint:chat", 0.0, None, None)
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

fn resolver() -> TurnModelResolver {
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
        .expect("turn models");
    let resolver = TurnModelResolver::from_turn_models(&models);
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
        .expect("turn models");
    let resolver = TurnModelResolver::from_turn_models(&models);
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
        .expect("turn models")
        .with_test_route("hint:burst", route.clone());
    let resolver = TurnModelResolver::from_turn_models(&models);

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
        .expect("turn models")
        .with_test_route("hint:burst", route.clone());
    let resolver = TurnModelResolver::from_turn_models(&models);

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
