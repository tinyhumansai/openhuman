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
