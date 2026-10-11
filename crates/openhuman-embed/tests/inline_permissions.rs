//! A host can await its own approval UI inline without polling the core.

mod common;

use common::{
    chat_completion, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion,
};
use openhuman_embed::seams::{ToolHookContext, ToolHookDecision};
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
    async fn execute(&self, _: Value) -> anyhow::Result<openhuman_core::tools::ToolResult> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(openhuman_core::tools::ToolResult::success("ran"))
    }
}
#[test]
fn inline_approval_waits_for_the_host_and_isolated_turn_policies_can_only_narrow() {
    runtime().block_on(async { tokio::spawn(scenario()).await.unwrap() });
}
async fn scenario() {
    let backend = stub_backend().await;
    let provider = scripted_provider(
        vec![
            tool_call_completion("probe", "{}"),
            tool_call_completion("probe", "{}"),
            tool_call_completion("probe", "{}"),
            chat_completion("done"),
            tool_call_completion("probe", "{}"),
            tool_call_completion("probe", "{}"),
        ],
        "done",
    )
    .await;
    let other_provider =
        scripted_provider(vec![tool_call_completion("probe", "{}")], "other done").await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
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
    let (requests, mut received) = tokio::sync::mpsc::unbounded_channel::<(
        ToolHookContext,
        tokio::sync::oneshot::Sender<ToolHookDecision>,
    )>();
    let agent = runtime
        .agent(
            spec("interactive", &provider, calls.clone()).can_use_tool(move |context| {
                let (answer, decision) = tokio::sync::oneshot::channel();
                requests.send((context.clone(), answer)).unwrap();
                Box::pin(async move {
                    decision
                        .await
                        .unwrap_or_else(|_| ToolHookDecision::Deny("UI closed".into()))
                })
            }),
        )
        .unwrap();
    let other = runtime
        .agent(
            spec("other", &other_provider, other_calls.clone())
                .can_use_tool(|_| Box::pin(async { ToolHookDecision::Proceed })),
        )
        .unwrap();
    let first = tokio::spawn(agent.turn("needs approval").send());
    let (context, answer) =
        tokio::time::timeout(std::time::Duration::from_secs(5), received.recv())
            .await
            .unwrap()
            .unwrap();
    assert_eq!(context.agent_id.as_deref(), Some("interactive"));
    assert_eq!(context.tool_name, "probe");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "tool ran before the UI answered"
    );
    other.run("independent worker").await.unwrap();
    assert_eq!(other_calls.load(Ordering::SeqCst), 1);
    answer
        .send(ToolHookDecision::Deny("human refused".into()))
        .unwrap();
    assert!(
        first.await.unwrap().is_err(),
        "denied call unexpectedly succeeded"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let second = tokio::spawn(
        agent
            .turn("turn-specific refusal")
            .can_use_tool(|_| {
                Box::pin(async { ToolHookDecision::Deny("turn narrowed authority".into()) })
            })
            .send(),
    );
    let (_, answer) = received.recv().await.unwrap();
    answer.send(ToolHookDecision::Proceed).unwrap();
    assert!(
        second.await.unwrap().is_err(),
        "turn denial unexpectedly succeeded"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "turn policy was skipped");
    let third = tokio::spawn(agent.turn("next turn").send());
    let (_, answer) = received.recv().await.unwrap();
    answer.send(ToolHookDecision::Proceed).unwrap();
    third.await.unwrap().unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "previous turn's refusal leaked"
    );
    let mut pending = agent.turn("cancel while the UI waits");
    let cancellation = pending.cancellation_handle();
    let cancelled = tokio::spawn(pending.send());
    let (_, answer) = received.recv().await.unwrap();
    cancellation.cancel().await;
    assert!(matches!(
        cancelled.await.unwrap(),
        Err(openhuman_embed::CoreError::TurnCancelled { .. })
    ));
    assert!(
        answer.is_closed(),
        "cancelled turn left an approval receiver alive"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let closed_ui = tokio::spawn(agent.turn("closed UI refuses").send());
    let (_, answer) = received.recv().await.unwrap();
    drop(answer);
    assert!(closed_ui.await.unwrap().is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "closing the UI granted permission"
    );
    repeated_permissions(&runtime).await;
}

async fn repeated_permissions(runtime: &Runtime) {
    let provider = scripted_provider(vec![tool_call_completion("probe", "{}")], "done").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let tool_calls = calls.clone();
    let agent = runtime
        .agent(
            AgentSpec::new("repeated-permissions")
                .provider(route(&provider, "fixture"))
                .definition(
                    AgentDefinitionSpec::new()
                        .bare_prompt("Use probe then answer")
                        .tools(ToolScopeSpec::HostOnly),
                )
                .tools(move |_| {
                    HostTurnTools::advertised(vec![Box::new(Probe(tool_calls.clone()))])
                })
                .can_use_tool(|_| {
                    Box::pin(async { ToolHookDecision::Deny("first policy refused".into()) })
                })
                .can_use_tool(|_| Box::pin(async { ToolHookDecision::Proceed })),
        )
        .unwrap();
    assert!(agent.run("perform probe").await.is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "later permission erased the earlier denial"
    );
}
