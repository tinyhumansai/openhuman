mod common;
use common::{chat_completion, chat_requests, offline_config, runtime, stub_backend};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, HostTools, HostTurnTools, Provider, Runtime, Tool,
    ToolScopeSpec, Workspace,
};
use std::sync::Arc;
use wiremock::matchers::{method, path};
use wiremock::{Mock, ResponseTemplate};
// Runtime is process-wide. Keep the guard on the synchronous test caller;
// it outlives block_on and the scoped Runtime/Agent teardown inside its future.
static RUNTIME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Marker(&'static str);
#[async_trait::async_trait]
impl Tool for Marker {
    fn exposure(&self) -> openhuman_embed::ToolExposure {
        openhuman_embed::ToolExposure::Deferred
    }
    fn name(&self) -> &str {
        self.0
    }
    fn description(&self) -> &str {
        "Send a message to the hive"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object", "properties":{"message":{"type":"string"}}, "required":["message"]})
    }
    async fn execute(
        &self,
        _: serde_json::Value,
    ) -> anyhow::Result<openhuman_core::tools::ToolResult> {
        Ok(openhuman_core::tools::ToolResult::success("accepted"))
    }
}
#[test]
fn attached_tools_survive_clones_and_session_resume() {
    let _runtime = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let collision = openhuman_core::agent::OpenHumanSessionHost::builder()
        .tools(vec![Box::new(Marker("mcp_fixture_send"))])
        .synthesized_tools(vec![Box::new(Marker("mcp_fixture_send"))])
        .permanent_tool_names(std::collections::HashSet::from(["mcp_fixture_send".into()]))
        .build()
        .err()
        .expect("a permanent source cannot shadow a synthesized MCP tool");
    assert!(collision
        .to_string()
        .contains("collision with synthesized tool"));
    runtime().block_on(async {
        tokio::spawn(async {
            let backend = stub_backend().await;
            let provider = wiremock::MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(ResponseTemplate::new(200).set_body_json(chat_completion("reply")))
                .mount(&provider)
                .await;
            let runtime = Runtime::builder()
                .config(offline_config())
                .workspace(Workspace::Ephemeral)
                .backend_url(backend.uri())
                .build()
                .await
                .unwrap();
            let agent = runtime
                .agent(
                    AgentSpec::new("attached")
                        .provider(
                            Provider::openai_compatible(
                                format!("{}/v1", provider.uri()),
                                "fixture",
                            )
                            .model("fixture"),
                        )
                        .definition(
                            AgentDefinitionSpec::new()
                                .system_prompt("FROZEN_HOST_PROMPT")
                                .tools(ToolScopeSpec::Named(vec![
                                    "shell".into(),
                                    "tool_search".into(),
                                ])),
                        ),
                )
                .unwrap();
            let original_config = agent.config().clone();
            assert!(agent
                .attach_tools("", Arc::new(|_| HostTurnTools::default()))
                .is_err());
            assert!(agent
                .attach_tools(
                    "builtin-collision",
                    Arc::new(|_| HostTurnTools::advertised(vec![Box::new(Marker("shell"))]))
                )
                .is_err());
            let peer = runtime.agent(AgentSpec::new("peer")).unwrap();
            assert!(!agent.same_agent(&peer));
            assert_eq!(agent.runtime_id(), peer.runtime_id());
            let clone = agent.clone();
            assert!(agent.same_agent(&clone));
            assert_eq!(agent.runtime_id(), clone.runtime_id());
            let factory: HostTools =
                Arc::new(|_| HostTurnTools::advertised(vec![Box::new(Marker("hivemind_message"))]));
            agent.attach_tools("tinyhivemind", factory.clone()).unwrap();
            clone.attach_tools("tinyhivemind", factory).unwrap();
            assert!(clone
                .attach_tools("tinyhivemind", Arc::new(|_| HostTurnTools::default()))
                .is_err());
            assert!(clone
                .attach_tools(
                    "collision",
                    Arc::new(|_| HostTurnTools::advertised(vec![Box::new(Marker(
                        "hivemind_message"
                    ))]))
                )
                .is_err());
            agent
                .turn("first user")
                .session("continuing")
                .send()
                .await
                .unwrap();
            clone
                .turn("second user")
                .session("continuing")
                .send()
                .await
                .unwrap();
            agent
                .attach_tools(
                    "second",
                    Arc::new(|_| {
                        HostTurnTools::advertised(vec![Box::new(Marker("hivemind_list"))])
                    }),
                )
                .unwrap();
            clone
                .turn("third user")
                .session("continuing")
                .send()
                .await
                .unwrap();
            clone
                .turn("fourth user")
                .session("continuing")
                .send()
                .await
                .unwrap();
            assert_eq!(agent.config().action_dir, original_config.action_dir);
            let paths = std::fs::read_dir(agent.transcripts_dir())
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>();
            assert!(paths
                .iter()
                .any(|path| path.to_string_lossy().contains(".g1.jsonl")));
            assert!(
                !paths
                    .iter()
                    .any(|path| path.to_string_lossy().contains(".g2.jsonl")),
                "identical catalogue must not create another generation"
            );
            let successor = paths
                .iter()
                .find(|path| path.to_string_lossy().contains(".g1.jsonl"))
                .unwrap();
            let durable = std::fs::read_to_string(successor).unwrap();
            assert!(
                durable
                    .lines()
                    .any(|line| line.contains("openhuman-permanent-tools")
                        && line.contains("hivemind_list")),
                "updated catalogue must be durable"
            );
            let requests = chat_requests(&provider).await;
            assert_eq!(requests.len(), 4);
            for (index, request) in requests.iter().enumerate() {
                let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
                let messages = body["messages"].as_array().unwrap();
                let system = messages
                    .iter()
                    .filter(|m| m["role"] == "system")
                    .map(|m| m["content"].as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join("\n");
                assert!(system.contains("FROZEN_HOST_PROMPT"));
                assert_eq!(system.matches("hivemind_message").count(), 1, "{system}");
                let tools = body["tools"].as_array().unwrap();
                assert_eq!(
                    tools
                        .iter()
                        .filter(|t| t["function"]["name"] == "hivemind_message")
                        .count(),
                    1,
                    "request {index}: {body}"
                );
                if index >= 2 {
                    assert_eq!(system.matches("hivemind_list").count(), 1);
                    assert_eq!(
                        tools
                            .iter()
                            .filter(|t| t["function"]["name"] == "hivemind_list")
                            .count(),
                        1
                    );
                }
                if index > 0 {
                    assert!(messages.iter().any(|m| m["role"] == "user"
                        && m["content"].as_str().unwrap_or("").contains("first user")));
                }
            }
        })
        .await
        .unwrap();
    });
}

#[test]
fn attaching_before_first_turn_preserves_and_executes_native_tools() {
    let _runtime = RUNTIME_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    runtime().block_on(async {
        tokio::spawn(async {
            struct NativeCall(std::sync::atomic::AtomicUsize);
            impl wiremock::Respond for NativeCall {
                fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
                    let body = if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                        serde_json::json!({
                            "id":"fresh-wildcard", "object":"chat.completion", "model":"fixture",
                            "choices":[{"index":0,"finish_reason":"tool_calls","message":{
                                "role":"assistant","content":null,"tool_calls":[{
                                    "id":"native-shell","type":"function","function":{
                                        "name":"shell","arguments":"{\"command\":\"printf native-wildcard-survived\"}"
                                    }
                                }]
                            }}]
                        })
                    } else {
                        chat_completion("native tool completed")
                    };
                    ResponseTemplate::new(200).set_body_json(body)
                }
            }
            let backend = stub_backend().await;
            let provider = wiremock::MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/v1/chat/completions"))
                .respond_with(NativeCall(std::sync::atomic::AtomicUsize::new(0)))
                .mount(&provider).await;
            let runtime = Runtime::builder()
                .config(offline_config()).workspace(Workspace::Ephemeral)
                .backend_url(backend.uri()).build().await.unwrap();
            let agent = runtime.agent(AgentSpec::new("fresh-attached")
                .provider(Provider::openai_compatible(format!("{}/v1",provider.uri()),"fixture").model("fixture"))
                .definition(AgentDefinitionSpec::new().system_prompt("ORIGINAL_FRESH_HOST_PROMPT")
                    .tools(ToolScopeSpec::Wildcard))).unwrap();
            let attachment: HostTools = Arc::new(|_| HostTurnTools::advertised(vec![Box::new(Marker("hivemind_message"))]));
            agent.attach_tools("hivemind", attachment).unwrap();
            agent.run("Run the native tool").await.unwrap();
            let requests = chat_requests(&provider).await;
            assert_eq!(requests.len(), 2);
            let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
            let names: Vec<_> = first["tools"].as_array().unwrap().iter()
                .filter_map(|tool| tool["function"]["name"].as_str()).collect();
            assert!(names.contains(&"shell"), "native wildcard tool missing: {names:?}");
            assert!(names.contains(&"hivemind_message"));
            let second: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
            assert!(second["messages"].as_array().unwrap().iter().any(|message|
                message["role"] == "tool" && message["content"].as_str().is_some_and(|text|
                    text.contains("native-wildcard-survived") && !text.contains("unknown tool"))),
                "the native executor must actually run: {second}");
            assert!(first["messages"].to_string().contains("ORIGINAL_FRESH_HOST_PROMPT"));
        }).await.unwrap();
    });
}

#[test]
fn permanent_metadata_requires_a_real_source() {
    let factory = HostTurnTools {
        permanent: std::collections::HashSet::from(["missing_source".into()]),
        ..Default::default()
    };
    assert!(!factory.is_empty(), "metadata must reach source validation");
}
