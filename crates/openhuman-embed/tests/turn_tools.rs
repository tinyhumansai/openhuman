//! Per-turn host-tool belts replace agent tools without leaking into other turns.

mod common;

use common::{
    chat_completion, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion,
};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, HostTurnTools, Runtime, Tool, ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

struct Probe(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "Perform a host operation"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    async fn execute(&self, _: Value) -> anyhow::Result<tinytools::ToolResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(tinytools::ToolResult::success("ran"))
    }
}
#[test]
fn turn_tool_belts_replace_agent_tools_without_leaking_into_resumed_or_concurrent_turns() {
    runtime().block_on(async { tokio::spawn(scenario()).await.unwrap() });
}
async fn scenario() {
    let backend = stub_backend().await;
    let provider = scripted_provider(
        (0..3)
            .flat_map(|_| [tool_call_completion("probe", "{}"), chat_completion("done")])
            .collect(),
        "done",
    )
    .await;
    let other_provider = scripted_provider(vec![tool_call_completion("probe", "{}")], "done").await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let replacement_calls = Arc::new(AtomicUsize::new(0));
    let other_calls = Arc::new(AtomicUsize::new(0));
    let spec = |id, provider: &wiremock::MockServer, calls: Arc<AtomicUsize>| {
        AgentSpec::new(id)
            .provider(route(provider, "fixture"))
            .definition(
                AgentDefinitionSpec::new()
                    .bare_prompt("Use probe then answer")
                    .tools(ToolScopeSpec::HostOnly),
            )
            .tools(move |_| HostTurnTools::advertised(vec![Box::new(Probe(calls.clone()))]))
    };
    let agent = runtime
        .agent(spec("replace", &provider, calls.clone()))
        .unwrap();
    let other = runtime
        .agent(spec("other", &other_provider, other_calls.clone()))
        .unwrap();
    let replacement = replacement_calls.clone();
    let (first, other_result) = tokio::join!(
        agent
            .turn("replacement")
            .session("same-thread")
            .tools(move |_| HostTurnTools::advertised(vec![Box::new(Probe(replacement.clone()))]))
            .send(),
        other.run("independent")
    );
    first.unwrap();
    other_result.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(replacement_calls.load(Ordering::SeqCst), 1);
    assert_eq!(other_calls.load(Ordering::SeqCst), 1);
    agent
        .turn("revoked")
        .session("same-thread")
        .tools(|_| HostTurnTools::advertised(vec![]))
        .send()
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        replacement_calls.load(Ordering::SeqCst),
        1,
        "revoked tool was still executable"
    );
    agent
        .turn("restore")
        .session("same-thread")
        .send()
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let requests = common::chat_requests(&provider).await;
    let revoked = requests
        .iter()
        .find(|r| {
            r.body_json::<Value>().unwrap()["messages"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| {
                    m["content"]
                        .as_str()
                        .is_some_and(|s| s.ends_with("revoked"))
                })
        })
        .unwrap();
    assert!(!revoked.body_json::<Value>().unwrap()["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|t| t["function"]["name"] == "probe")));
    // A mixed builtin/host definition must also discard historical declarations
    // on an explicit replacement. HostOnly's inherent strict snapshot is not
    // enough to exercise the replacement's dispatch scope.
    let mixed_provider = scripted_provider(
        vec![
            tool_call_completion("probe", "{}"),
            chat_completion("done"),
            chat_completion("no tools"),
            tool_call_completion("probe", "{}"),
            chat_completion("done"),
        ],
        "done",
    )
    .await;
    let mixed_calls = Arc::new(AtomicUsize::new(0));
    let mixed = runtime
        .agent(
            spec("mixed", &mixed_provider, mixed_calls.clone()).definition(
                AgentDefinitionSpec::new()
                    .bare_prompt("Use the offered tools")
                    .tools(ToolScopeSpec::Named(vec!["probe".into()])),
            ),
        )
        .unwrap();
    mixed
        .turn("initial belt")
        .session("mixed-thread")
        .send()
        .await
        .unwrap();
    mixed
        .turn("replace with empty belt")
        .session("mixed-thread")
        .tools(|_| HostTurnTools::advertised(vec![]))
        .send()
        .await
        .unwrap();
    assert_eq!(mixed_calls.load(Ordering::SeqCst), 1);
    let requests = common::chat_requests(&mixed_provider).await;
    assert!(!requests[2].body_json::<Value>().unwrap()["tools"]
        .as_array()
        .is_some_and(|tools| tools.iter().any(|t| t["function"]["name"] == "probe")));
    mixed
        .turn("restore belt")
        .session("mixed-thread")
        .send()
        .await
        .unwrap();
    assert_eq!(mixed_calls.load(Ordering::SeqCst), 2);
}
