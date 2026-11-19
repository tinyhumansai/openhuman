//! A host can await its own approval UI inline without polling the core.

mod common;

use common::{
    chat_completion, chat_requests, offline_config, route, runtime, scripted_provider,
    stub_backend, tool_call_completion, tool_results,
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

type Approval = (
    ToolHookContext,
    tokio::sync::oneshot::Sender<ToolHookDecision>,
);

/// The longest the host waits for any one step of a turn. Every wait in the
/// scenario goes through here so a regression fails naming the step that never
/// arrived instead of hanging until CI cancels the job.
const STEP: std::time::Duration = std::time::Duration::from_secs(30);

/// Fail the test naming `reason`. The core's panic hook reports only the
/// location of a panic, so the message goes to stderr first.
fn fail(reason: String) -> ! {
    eprintln!("inline_permissions: {reason}");
    panic!("{reason}");
}

async fn within<T>(step: &str, wait: impl std::future::Future<Output = T>) -> T {
    match tokio::time::timeout(STEP, wait).await {
        Ok(value) => value,
        Err(_) => fail(format!("timed out after {STEP:?} waiting for {step}")),
    }
}

async fn next_approval(
    received: &mut tokio::sync::mpsc::UnboundedReceiver<Approval>,
    step: &str,
) -> Approval {
    match within(step, received.recv()).await {
        Some(approval) => approval,
        None => fail(format!("approval channel closed while waiting for {step}")),
    }
}

/// A refusal that ends the turn. TinyAgents answers a plain `Deny` to the model
/// and lets the turn continue (so the model may ask for the tool again, and the
/// host is asked again); only `continue_ = Some(false)` stops the run. These
/// scenarios script exactly one model call per refused turn, so they refuse
/// with a stop.
fn refuse_and_stop(reason: &str) -> ToolHookDecision {
    ToolHookDecision {
        continue_: Some(false),
        ..ToolHookDecision::deny(reason)
    }
}

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
            .tools(move |_| {
                let calls = calls.clone();
                Box::pin(async move { HostTurnTools::advertised(vec![Box::new(Probe(calls))]) })
            })
    };
    let (requests, mut received) = tokio::sync::mpsc::unbounded_channel::<Approval>();
    let agent = runtime
        .agent(
            spec("interactive", &provider, calls.clone()).can_use_tool(move |context| {
                let (answer, decision) = tokio::sync::oneshot::channel();
                requests.send((context.clone(), answer)).unwrap();
                Box::pin(async move {
                    decision
                        .await
                        .unwrap_or_else(|_| refuse_and_stop("UI closed"))
                })
            }),
        )
        .unwrap();
    let other = runtime
        .agent(
            spec("other", &other_provider, other_calls.clone())
                .can_use_tool(|_| Box::pin(async { ToolHookDecision::default() })),
        )
        .unwrap();
    let first = tokio::spawn(agent.turn("needs approval").send());
    let (context, answer) = next_approval(&mut received, "the first approval request").await;
    assert_eq!(context.agent_id.as_deref(), Some("interactive"));
    assert_eq!(context.tool_name, "probe");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "tool ran before the UI answered"
    );
    within(
        "the independent agent to finish its turn",
        other.run("independent worker"),
    )
    .await
    .unwrap();
    assert_eq!(other_calls.load(Ordering::SeqCst), 1);
    answer.send(refuse_and_stop("human refused")).unwrap();
    assert!(
        within("the denied first turn to finish", first)
            .await
            .unwrap()
            .is_err(),
        "denied call unexpectedly succeeded"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let second = tokio::spawn(
        agent
            .turn("turn-specific refusal")
            .can_use_tool(|_| Box::pin(async { refuse_and_stop("turn narrowed authority") }))
            .send(),
    );
    let (_, answer) = next_approval(
        &mut received,
        "the approval request of the turn-narrowed turn",
    )
    .await;
    answer.send(ToolHookDecision::default()).unwrap();
    assert!(
        within("the turn-narrowed turn to finish", second)
            .await
            .unwrap()
            .is_err(),
        "turn denial unexpectedly succeeded"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0, "turn policy was skipped");
    let third = tokio::spawn(agent.turn("next turn").send());
    let (_, answer) = next_approval(&mut received, "the approval request of the next turn").await;
    answer.send(ToolHookDecision::default()).unwrap();
    within("the approved next turn to finish", third)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "previous turn's refusal leaked"
    );
    let mut pending = agent.turn("cancel while the UI waits");
    let cancellation = pending.cancellation_handle();
    let cancelled = tokio::spawn(pending.send());
    let (_, answer) =
        next_approval(&mut received, "the approval request of the cancelled turn").await;
    within("the cancellation to complete", cancellation.cancel()).await;
    assert!(matches!(
        within("the cancelled turn to finish", cancelled)
            .await
            .unwrap(),
        Err(openhuman_embed::CoreError::TurnCancelled { .. })
    ));
    assert!(
        answer.is_closed(),
        "cancelled turn left an approval receiver alive"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let closed_ui = tokio::spawn(agent.turn("closed UI refuses").send());
    let (_, answer) =
        next_approval(&mut received, "the approval request of the closed-UI turn").await;
    drop(answer);
    assert!(within("the closed-UI turn to finish", closed_ui)
        .await
        .unwrap()
        .is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "closing the UI granted permission"
    );
    within(
        "the repeated-permissions scenario",
        repeated_permissions(&runtime),
    )
    .await;
    within(
        "the plain-denial scenario",
        plain_denial_is_answered_to_the_model(&runtime),
    )
    .await;
}

/// A `Deny` without `continue_ = Some(false)` does not end the turn: the tool
/// never runs, the model is told why, and it answers on its own.
async fn plain_denial_is_answered_to_the_model(runtime: &Runtime) {
    let provider = scripted_provider(vec![tool_call_completion("probe", "{}")], "answered").await;
    let calls = Arc::new(AtomicUsize::new(0));
    let tool_calls = calls.clone();
    let agent = runtime
        .agent(
            AgentSpec::new("plain-denial")
                .provider(route(&provider, "fixture"))
                .definition(
                    AgentDefinitionSpec::new()
                        .bare_prompt("Use probe then answer")
                        .tools(ToolScopeSpec::HostOnly),
                )
                .tools(move |_| {
                    let tool_calls = tool_calls.clone();
                    Box::pin(
                        async move { HostTurnTools::advertised(vec![Box::new(Probe(tool_calls))]) },
                    )
                })
                .can_use_tool(|_| Box::pin(async { ToolHookDecision::deny("plain refusal") })),
        )
        .unwrap();
    within(
        "the plain-denial turn to finish",
        agent.run("perform probe"),
    )
    .await
    .expect("a plain denial is answered to the model, not fatal to the turn");
    assert_eq!(calls.load(Ordering::SeqCst), 0, "a denied tool ran");
    let requests = chat_requests(&provider).await;
    assert_eq!(
        requests.len(),
        2,
        "the model was not asked again after the denial"
    );
    assert!(
        tool_results(&requests[1]).contains("plain refusal"),
        "the model was not told why the tool was refused"
    );
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
                    let tool_calls = tool_calls.clone();
                    Box::pin(
                        async move { HostTurnTools::advertised(vec![Box::new(Probe(tool_calls))]) },
                    )
                })
                .can_use_tool(|_| Box::pin(async { refuse_and_stop("first policy refused") }))
                .can_use_tool(|_| Box::pin(async { ToolHookDecision::default() })),
        )
        .unwrap();
    assert!(within(
        "the repeated-permissions turn to finish",
        agent.run("perform probe")
    )
    .await
    .is_err());
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "later permission erased the earlier denial"
    );
}
