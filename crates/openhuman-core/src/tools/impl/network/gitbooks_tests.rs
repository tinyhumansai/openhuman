use super::*;

#[cfg(feature = "modules")]
use tinybus::module::ModuleClient;

#[cfg(feature = "modules")]
#[derive(Clone, Default)]
struct MockProviderState {
    calls: std::sync::Arc<std::sync::Mutex<Vec<Value>>>,
}

#[cfg(feature = "modules")]
#[derive(Clone)]
struct MockTinyMcp {
    provider_state: MockProviderState,
}

#[cfg(feature = "modules")]
#[tinybus::interface(name = "ai.tinyhumans.tinymcp.Mcp")]
impl MockTinyMcp {
    #[tinybus(confidential)]
    async fn configured_tool_call(
        &self,
        request: tinymcp_bus::ConfiguredToolCallRequest,
    ) -> tinybus::Result<tinymcp_bus::ClassifiedStaticToolCall> {
        let tool = request.tool.clone();
        let server = request.server.name.clone();
        self.provider_state
            .calls
            .lock()
            .expect("provider request log")
            .push(serde_json::json!({
                "tool": tool,
                "server": server,
                "endpoint": request.server.endpoint,
                "timeout_secs": request.server.timeout_secs,
                "arguments": request.arguments,
            }));
        let text = match tool.as_str() {
            "searchDocumentation" => "search fixture result",
            "getPage" => "page fixture markdown",
            _ => "unexpected tool",
        };
        let rendered = tinymcp_bus::McpToolResult::success(text);
        let outcome = tinymcp_bus::McpCallOutcome::answered(&server, &tool);
        let envelope = tinymcp_bus::McpResultEnvelope::new(&server, &tool, &rendered)
            .with_outcome(outcome.clone());
        Ok(tinymcp_bus::ClassifiedStaticToolCall {
            outcome,
            rendered: Some(rendered),
            envelope: Some(envelope),
            error_message: None,
        })
    }
}

#[cfg(feature = "modules")]
async fn mock_module_client(
    state: MockProviderState,
) -> (ModuleClient, tinybus::Connection, MockProviderState) {
    use tinybus::{broker::Broker, transport::memory::MemoryBus, Connection, ObjectPath};

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.expect("service transport"))
        .await
        .expect("service connection");
    let caller = Connection::connect(bus.connect().await.expect("caller transport"))
        .await
        .expect("caller connection");
    let provider_state = state.clone();
    service
        .serve_at(
            ObjectPath::new(tinymcp_bus::names::OBJECT_PATH).expect("TinyMCP object path"),
            MockTinyMcp {
                provider_state: state,
            },
        )
        .await
        .expect("serve TinyMCP fixture");
    service
        .request_name(tinymcp_bus::names::INTERFACE)
        .await
        .expect("TinyMCP bus name");
    let proxy = caller
        .proxy(
            tinymcp_bus::names::INTERFACE,
            tinymcp_bus::names::OBJECT_PATH,
            tinymcp_bus::names::INTERFACE,
        )
        .expect("TinyMCP proxy");
    (
        crate::modules::ops::fixture_client(proxy),
        service,
        provider_state,
    )
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn gitbooks_tools_fail_closed_when_the_module_is_not_attested() {
    let state = MockProviderState::default();
    let (client, _service, module_calls) = mock_module_client(state).await;
    let endpoint = "https://docs.example.test/path?token=secret".to_owned();
    let search = GitbooksSearchTool::with_client(endpoint.clone(), 5, client.clone());
    let page = GitbooksGetPageTool::with_client(endpoint.clone(), 5, client);

    let search_result = search
        .execute(serde_json::json!({"query": "module migration"}))
        .await
        .expect("search executes");
    let page_result = page
        .execute(serde_json::json!({"url": "https://docs.example.test/guide"}))
        .await
        .expect("page fetch executes");

    assert!(search_result.is_error, "{}", search_result.output());
    assert!(search_result
        .output()
        .contains("MODULE_CALL_REPORTED: module transport failed"));
    assert!(page_result.is_error, "{}", page_result.output());
    assert!(page_result
        .output()
        .contains("MODULE_CALL_REPORTED: module transport failed"));
    assert!(
        module_calls
            .calls
            .lock()
            .expect("provider request log")
            .is_empty(),
        "an unattested in-memory service must never receive confidential module calls"
    );
}

#[test]
fn module_rendered_results_preserve_text_json_error_and_markdown() {
    let envelope = tinymcp_bus::McpResultEnvelope {
        kind: tinymcp_bus::MCP_RESULT_KIND.to_owned(),
        server: "openhuman-gitbooks".to_owned(),
        tool: "searchDocumentation".to_owned(),
        structured_content: Some(json!({"title": "Guide", "rank": 1})),
        meta: Some(json!({"source": "fixture"})),
        resources: vec![tinymcp_bus::McpResourceContents {
            uri: "ui://guide".to_owned(),
            mime_type: Some("text/markdown".to_owned()),
            text: Some("# Guide".to_owned()),
            blob: None,
            meta: None,
        }],
        outcome: None,
    };
    let result = map_module_rendered_result(
        tinymcp_bus::McpToolResult {
            content: vec![
                tinymcp_bus::McpToolContent::Text {
                    text: "plain excerpt".to_owned(),
                },
                tinymcp_bus::McpToolContent::Json {
                    data: json!({"title": "Guide", "rank": 1}),
                },
            ],
            is_error: true,
            markdown_formatted: Some("**formatted excerpt**".to_owned()),
            ..Default::default()
        },
        Some(envelope.clone()),
    );

    assert!(result.is_error);
    assert_eq!(
        result.markdown_formatted.as_deref(),
        Some("**formatted excerpt**")
    );
    assert_eq!(result.content.len(), 2);
    assert!(matches!(
        &result.content[0],
        tinytools::ToolContent::Text { text } if text == "plain excerpt"
    ));
    assert!(matches!(
        &result.content[1],
        tinytools::ToolContent::Json { data } if data == &json!({"title": "Guide", "rank": 1})
    ));
    assert_eq!(result.metadata, Some(envelope.to_metadata()));
}

#[tokio::test]
async fn search_rejects_empty_query() {
    let t = GitbooksSearchTool::new(
        "https://example.com/mcp".into(),
        5,
        crate::config::Config::default(),
    )
    .expect("a tool builds");
    let result = t.execute(json!({"query": "   "})).await.unwrap();
    assert!(result.is_error);
    assert!(result.output().contains("empty"));
}

#[tokio::test]
async fn get_page_rejects_empty_url() {
    let t = GitbooksGetPageTool::new(
        "https://example.com/mcp".into(),
        5,
        crate::config::Config::default(),
    )
    .expect("a tool builds");
    let result = t.execute(json!({"url": ""})).await.unwrap();
    assert!(result.is_error);
}

#[tokio::test]
async fn live_search_smoke() {
    if std::env::var("OPENHUMAN_GITBOOKS_LIVE_TEST")
        .ok()
        .as_deref()
        != Some("1")
    {
        return;
    }
    let t = GitbooksSearchTool::new(
        "https://tinyhumans.gitbook.io/openhuman/~gitbook/mcp".into(),
        30,
        crate::config::Config::default(),
    )
    .expect("a tool builds");
    let result = t
        .execute(json!({"query": "what is openhuman"}))
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "live search returned error: {}",
        result.output()
    );
    assert!(!result.output().is_empty());
}
