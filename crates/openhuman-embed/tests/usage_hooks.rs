//! Scoped usage observers and budget policies run between provider calls.

mod common;

use common::{
    chat_completion, chat_requests, offline_config, route, runtime, scripted_provider,
    stub_backend, tool_call_completion,
};
use openhuman_embed::seams::{StopDecision, StopHook, TurnCost, TurnState};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, HostTurnTools, Runtime, Tool, ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

type Seen = Arc<Mutex<Vec<(String, u32, u64, Option<f64>)>>>;
struct Policy {
    label: &'static str,
    seen: Seen,
    stop: bool,
}
#[async_trait::async_trait]
impl StopHook<TurnCost> for Policy {
    fn name(&self) -> &str {
        "usage-budget"
    }
    async fn check(&self, state: &TurnState<'_>) -> StopDecision {
        self.seen.lock().unwrap().push((
            self.label.into(),
            state.iteration,
            state.cost.input_tokens + state.cost.output_tokens,
            state.cost.total_usd(),
        ));
        if self.stop {
            StopDecision::Stop {
                reason: "worker budget exhausted".into(),
            }
        } else {
            StopDecision::Continue
        }
    }
}
struct Probe;
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "Read a value"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{}})
    }
    async fn execute(&self, _: Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("value"))
    }
}
fn billed(mut response: Value) -> Value {
    response["usage"]["cost"] = json!(0.02);
    response
}
#[test]
fn per_agent_budget_stops_before_the_next_model_call_and_turn_observers_do_not_leak() {
    runtime().block_on(async { tokio::spawn(scenario()).await.unwrap() });
}
async fn scenario() {
    let backend = stub_backend().await;
    let a_provider = scripted_provider(
        vec![billed(tool_call_completion("probe", "{}"))],
        "extra call",
    )
    .await;
    let b_provider = scripted_provider(
        vec![
            billed(tool_call_completion("probe", "{}")),
            billed(chat_completion("done")),
            {
                let mut response = chat_completion("free");
                response["usage"]["cost"] = json!(0.0);
                response
            },
        ],
        "next turn",
    )
    .await;
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .build()
        .await
        .unwrap();
    let seen: Seen = Default::default();
    let spec = |id, provider: &wiremock::MockServer| {
        AgentSpec::new(id)
            .provider(route(provider, "fixture"))
            .definition(
                AgentDefinitionSpec::new()
                    .bare_prompt("Use probe then answer")
                    .tools(ToolScopeSpec::HostOnly),
            )
            .tools(|_| Box::pin(async { HostTurnTools::advertised(vec![Box::new(Probe)]) }))
    };
    let a = runtime
        .agent(spec("budget-a", &a_provider).stop_hook(Arc::new(Policy {
            label: "a",
            seen: seen.clone(),
            stop: true,
        })))
        .unwrap();
    let b = runtime.agent(spec("budget-b", &b_provider)).unwrap();
    let (a_result, b_result) = tokio::join!(
        a.turn("work")
            .stop_hook(Arc::new(Policy {
                label: "a-turn",
                seen: seen.clone(),
                stop: false,
            }))
            .send(),
        b.turn("work")
            .stop_hook(Arc::new(Policy {
                label: "b-turn",
                seen: seen.clone(),
                stop: false
            }))
            .send()
    );
    assert_eq!(a_result.unwrap().usage.unwrap().cost_usd, Some(0.02));
    let b_result = b_result.unwrap();
    assert_eq!(
        chat_requests(&a_provider).await.len(),
        1,
        "budget permitted another model call"
    );
    assert_eq!(chat_requests(&b_provider).await.len(), 2);
    assert_eq!(b_result.usage.unwrap().cost_usd, Some(0.04));
    assert_eq!(
        b.run("a later turn").await.unwrap().usage.unwrap().cost_usd,
        Some(0.0)
    );
    assert_eq!(
        b.run("an unpriced turn")
            .await
            .unwrap()
            .usage
            .unwrap()
            .cost_usd,
        None
    );
    let seen = seen.lock().unwrap();
    assert_eq!(
        seen.iter().filter(|(label, _, _, _)| label == "a").count(),
        1
    );
    assert_eq!(
        seen.iter()
            .filter(|(label, _, _, _)| label == "a-turn")
            .count(),
        1,
        "agent budget stop skipped the turn usage observer"
    );
    let b_seen: Vec<_> = seen
        .iter()
        .filter(|(label, _, _, _)| label == "b-turn")
        .collect();
    assert_eq!(b_seen.len(), 2, "turn observer leaked to a later turn");
    assert_eq!(b_seen[0].1, 1);
    assert_eq!(b_seen[0].2, 2);
    assert_eq!(b_seen[0].3, Some(0.02));
    assert_eq!(b_seen[1].1, 2);
    assert_eq!(b_seen[1].2, 4);
    assert_eq!(b_seen[1].3, Some(0.04));
}
