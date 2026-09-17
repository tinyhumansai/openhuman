use super::*;
use crate::agent::tinyagents::{
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

struct QwenToolStreamModel;

#[async_trait]
impl ChatModel<()> for QwenToolStreamModel {
    async fn invoke(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelResponse> {
        Ok(ModelResponse::assistant("unused"))
    }

    async fn stream(
        &self,
        _state: &(),
        _request: ModelRequest,
    ) -> tinyinference::Result<ModelStream> {
        let completed = ModelResponse::assistant(
            r#"<tool_call>{"web_fetch","arguments":{"url":"https://example.com"}}</tool_call>"#,
        );
        Ok(Box::pin(futures::stream::iter(vec![
            ModelStreamItem::Started,
            ModelStreamItem::MessageDelta(MessageDelta::text("<tool_call>")),
            ModelStreamItem::Completed(completed),
        ])))
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

fn tool_schema(name: &str) -> tinyinference::tool::ToolSchema {
    tinyinference::tool::ToolSchema {
        name: name.to_string(),
        description: String::new(),
        parameters: serde_json::json!({"type": "object"}),
        format: Default::default(),
    }
}

fn url_tool_schema(name: &str) -> tinyinference::tool::ToolSchema {
    tinyinference::tool::ToolSchema::new(
        name,
        "",
        serde_json::json!({
            "type": "object",
            "properties": {"url": {"type": "string"}},
            "required": ["url"]
        }),
    )
}

#[test]
fn qwen_bare_name_repair_accepts_an_advertised_tool() {
    let response = repair_qwen_bare_name_tool_call(
        ModelResponse::assistant(
            r#"<tool_call>{"web_fetch", "arguments": {"url": "https://example.com", "max_bytes": 6000}}</tool_call>"#,
        ),
        &[tool_schema("web_fetch")],
    );

    assert_eq!(response.text(), "");
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].name, "web_fetch");
    assert_eq!(
        response.tool_calls()[0].arguments,
        serde_json::json!({"url": "https://example.com", "max_bytes": 6000})
    );
}

#[test]
fn qwen_bare_name_repair_rejects_an_unadvertised_tool() {
    let text = r#"<tool_call>{"shell", "arguments": {"command": "whoami"}}</tool_call>"#;
    let response = repair_qwen_bare_name_tool_call(
        ModelResponse::assistant(text),
        &[tool_schema("web_fetch")],
    );

    assert!(response.tool_calls().is_empty());
    assert!(!response.text().contains("<tool_call>"));
    assert!(response.text().contains("invalid tool call"));
}

#[test]
fn qwen_missing_name_repair_accepts_one_schema_valid_read_only_tool() {
    let response = repair_qwen_bare_name_tool_call(
        ModelResponse::assistant(
            r#"<tool_call>{"arguments":{"url":"https://example.com"}}</tool_call>"#,
        ),
        &[url_tool_schema("web_fetch"), tool_schema("web_search_tool")],
    );

    assert_eq!(response.text(), "");
    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].name, "web_fetch");
}

#[test]
fn qwen_missing_name_repair_rejects_ambiguous_or_acting_tools() {
    let text = r#"<tool_call>{"arguments":{"url":"https://example.com"}}</tool_call>"#;
    let ambiguous = repair_qwen_bare_name_tool_call(
        ModelResponse::assistant(text),
        &[
            url_tool_schema("web_fetch"),
            url_tool_schema("browser_open"),
        ],
    );
    assert!(ambiguous.tool_calls().is_empty());
    assert!(!ambiguous.text().contains("<tool_call>"));
    assert!(ambiguous.text().contains("invalid tool call"));

    let acting = repair_qwen_bare_name_tool_call(
        ModelResponse::assistant(text),
        &[url_tool_schema("shell")],
    );
    assert!(acting.tool_calls().is_empty());
    assert!(!acting.text().contains("<tool_call>"));
    assert!(acting.text().contains("invalid tool call"));
}

#[test]
fn qwen_normalization_admits_only_the_first_tool_call() {
    let mut response = ModelResponse::assistant("");
    response.message.tool_calls = vec![
        TaToolCall::new(
            "one",
            "web_search_tool",
            serde_json::json!({"query": "first"}),
        ),
        TaToolCall::new(
            "two",
            "web_search_tool",
            serde_json::json!({"query": "second"}),
        ),
    ];

    let response = normalize_qwen_tool_response(response, &[tool_schema("web_search_tool")]);

    assert_eq!(response.tool_calls().len(), 1);
    assert_eq!(response.tool_calls()[0].id, "one");
}

#[tokio::test]
async fn qwen_stream_hides_prompt_tool_markup_and_keeps_completed_call() {
    use futures::StreamExt;

    let route = RouteRecordingModel::new(
        Arc::new(QwenToolStreamModel),
        "lmstudio",
        "qwen38-openhuman",
    );
    let request = ModelRequest::default().with_tools(vec![url_tool_schema("web_fetch")]);

    let items: Vec<_> = route
        .stream(&(), request)
        .await
        .expect("stream")
        .collect()
        .await;

    assert!(items.iter().all(|item| !matches!(
        item,
        ModelStreamItem::MessageDelta(delta) if delta.text.contains("<tool_call>")
    )));
    assert!(items.iter().any(|item| matches!(
        item,
        ModelStreamItem::Completed(response) if response.tool_calls().len() == 1
    )));
}
