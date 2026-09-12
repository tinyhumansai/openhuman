use super::*;
use crate::openhuman::agent::tinyagents::{
    current_resolved_provider_route, with_resolved_provider_route_scope, ResolvedProviderRoute,
};

struct SuccessfulModel;

#[async_trait]
impl ChatModel<()> for SuccessfulModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant("ok"))
    }
}

#[tokio::test]
async fn selected_model_records_concrete_route_and_fallback_overwrites_primary() {
    let primary = RouteRecordingModel::new(Arc::new(SuccessfulModel), "openhuman", "chat-v1");
    let fallback =
        RouteRecordingModel::new(Arc::new(SuccessfulModel), "anthropic", "claude-sonnet-4");

    let observed = with_resolved_provider_route_scope(async {
        primary
            .invoke(&(), ModelRequest::default())
            .await
            .expect("primary dispatch");
        fallback
            .invoke(&(), ModelRequest::default())
            .await
            .expect("fallback dispatch");
        current_resolved_provider_route()
    })
    .await;

    assert_eq!(
        observed,
        Some(ResolvedProviderRoute {
            provider: "anthropic".to_string(),
            model: "claude-sonnet-4".to_string(),
        })
    );
}

#[tokio::test]
async fn streamed_model_records_concrete_route_before_stream_consumption() {
    let model = RouteRecordingModel::new(Arc::new(SuccessfulModel), "openhuman", "reasoning-v1");

    let observed = with_resolved_provider_route_scope(async {
        let _stream = model
            .stream(&(), ModelRequest::default())
            .await
            .expect("stream dispatch");
        current_resolved_provider_route()
    })
    .await;

    assert_eq!(
        observed,
        Some(ResolvedProviderRoute {
            provider: "openhuman".to_string(),
            model: "reasoning-v1".to_string(),
        })
    );
}

/// A model that identifies itself for response-cache scoping.
struct IdentifiedModel;

#[async_trait]
impl ChatModel<()> for IdentifiedModel {
    fn cache_identity(&self) -> Option<String> {
        Some("test-provider:https://example/v1:model-x".to_string())
    }

    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant("ok"))
    }
}

/// The harness scopes its response cache on `ChatModel::cache_identity`, and
/// the trait default declines identity. Every host wrapper therefore has to
/// forward it, or every wrapped model collapses into the crate's
/// "anonymous-model" key space and a shared cache can cross-serve answers
/// between a hosted and a local model.
#[test]
fn every_turn_wrapper_forwards_the_inner_cache_identity() {
    let expected = Some("test-provider:https://example/v1:model-x".to_string());
    let inner: Arc<dyn ChatModel<()>> = Arc::new(IdentifiedModel);

    let route = RouteRecordingModel::new(inner.clone(), "p", "m");
    assert_eq!(route.cache_identity(), expected);

    let profile = ProfileOverrideModel::new(inner.clone(), ModelProfile::default());
    assert_eq!(profile.cache_identity(), expected);

    let capped = MaxTokensModel::new(inner.clone(), 1024);
    assert_eq!(capped.cache_identity(), expected);

    // Nested the way the turn builder stacks them.
    let stacked = RouteRecordingModel::new(
        Arc::new(MaxTokensModel::new(
            Arc::new(ProfileOverrideModel::new(inner, ModelProfile::default())),
            1024,
        )),
        "p",
        "m",
    );
    assert_eq!(stacked.cache_identity(), expected);
}

#[test]
fn profile_override_cache_identity_includes_request_model() {
    let inner: Arc<dyn ChatModel<()>> = Arc::new(IdentifiedModel);
    let first = ProfileOverrideModel::new(inner.clone(), ModelProfile::default())
        .with_request_model("model-a");
    let second =
        ProfileOverrideModel::new(inner, ModelProfile::default()).with_request_model("model-b");

    assert_ne!(first.cache_identity(), second.cache_identity());
}
