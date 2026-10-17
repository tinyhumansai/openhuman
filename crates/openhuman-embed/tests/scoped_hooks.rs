//! Agent and turn hooks stay isolated while concurrent workers share a runtime.

mod common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use common::{
    chat_completion, eventually, offline_config, route, runtime, scripted_provider, stub_backend,
    tool_call_completion,
};
use openhuman_embed::seams::{PostTurnHook, ToolHook, ToolHookContext, TurnContext};
use openhuman_embed::{
    AgentDefinitionSpec, AgentSpec, HostTurnTools, Runtime, Tool, ToolScopeSpec, Workspace,
};
use serde_json::{json, Value};
use tokio::sync::Barrier;
use wiremock::MockServer;

type Events = Arc<Mutex<Vec<(String, String, String)>>>;

struct Observer {
    label: &'static str,
    events: Events,
    first: AtomicBool,
    overlap: Option<Arc<Barrier>>,
}

impl Observer {
    fn new(label: &'static str, events: &Events, overlap: Option<Arc<Barrier>>) -> Arc<Self> {
        Arc::new(Self {
            label,
            events: events.clone(),
            first: AtomicBool::new(true),
            overlap,
        })
    }
    fn record(&self, phase: &str, owner: &str) {
        self.events
            .lock()
            .unwrap()
            .push((self.label.into(), phase.into(), owner.into()));
    }
}

#[async_trait::async_trait]
impl ToolHook for Observer {
    fn name(&self) -> &str {
        "same-policy-name"
    }
    async fn before_tool(&self, ctx: &ToolHookContext) -> anyhow::Result<()> {
        self.record("before", ctx.arguments["owner"].as_str().unwrap());
        if self.first.swap(false, Ordering::SeqCst) {
            if let Some(barrier) = &self.overlap {
                tokio::time::timeout(std::time::Duration::from_secs(10), barrier.wait()).await?;
            }
        }
        Ok(())
    }
    async fn after_tool(&self, ctx: &ToolHookContext) -> anyhow::Result<()> {
        assert_eq!(ctx.success, Some(true));
        self.record("after", ctx.arguments["owner"].as_str().unwrap());
        Ok(())
    }
}

#[async_trait::async_trait]
impl PostTurnHook for Observer {
    fn name(&self) -> &str {
        "same-policy-name"
    }
    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        assert!(ctx.session_id.is_some());
        assert!(ctx.agent_id.is_some());
        self.record("turn", ctx.user_message.lines().last().unwrap());
        Ok(())
    }
}

struct Probe;
#[async_trait::async_trait]
impl Tool for Probe {
    fn name(&self) -> &str {
        "probe"
    }
    fn description(&self) -> &str {
        "Report a worker's identity"
    }
    fn parameters_schema(&self) -> Value {
        json!({"type":"object","properties":{"owner":{"type":"string"}},"required":["owner"]})
    }
    async fn execute(&self, _: Value) -> anyhow::Result<tinytools::ToolResult> {
        Ok(tinytools::ToolResult::success("observed"))
    }
}

fn spec(id: &str, provider: &MockServer) -> AgentSpec {
    AgentSpec::new(id)
        .provider(route(provider, "fixture"))
        .definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Use probe once, then answer.")
                .tools(ToolScopeSpec::HostOnly),
        )
        .tools(|_| HostTurnTools::advertised(vec![Box::new(Probe)]))
}

async fn worker_provider(owner: &str) -> MockServer {
    scripted_provider(
        (0..4)
            .flat_map(|_| {
                [
                    tool_call_completion("probe", &json!({"owner":owner}).to_string()),
                    chat_completion("done"),
                ]
            })
            .collect(),
        "done",
    )
    .await
}

#[test]
fn hooks_are_additive_and_isolated_across_agents_turns_and_reused_ids() {
    runtime().block_on(async { tokio::spawn(scenario()).await.expect("scenario") });
}

async fn scenario() {
    let backend = stub_backend().await;
    let (provider_a, provider_b) = tokio::join!(worker_provider("a"), worker_provider("b"));
    let events: Events = Default::default();
    let global = Observer::new("global", &events, None);
    let runtime = Runtime::builder()
        .config(offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .tool_hook(global.clone())
        .post_turn_hook(global)
        .build()
        .await
        .expect("runtime");
    let overlap = Arc::new(Barrier::new(2));
    let hook_a = Observer::new("agent-a", &events, Some(overlap.clone()));
    let hook_b = Observer::new("agent-b", &events, Some(overlap));
    let a = runtime
        .agent(
            spec("worker-a", &provider_a)
                .tool_hook(hook_a.clone())
                .post_turn_hook(hook_a),
        )
        .unwrap();
    let b = runtime
        .agent(
            spec("worker-b", &provider_b)
                .tool_hook(hook_b.clone())
                .post_turn_hook(hook_b),
        )
        .unwrap();
    let turn_hook = Observer::new("turn-a", &events, None);
    let scheduled = openhuman_core::agent::host_agents::resolve("worker-a").unwrap();
    scheduled
        .scope(async {
            assert_eq!(
                openhuman_core::agent::hooks::embedder_tool_hooks_in(
                    &openhuman_core::core::runtime::DispatchCtx::ambient()
                )
                .len(),
                2,
                "scheduled turns lost the agent's tool hooks"
            );
            assert_eq!(
                openhuman_core::agent::hooks::embedder_post_turn_hooks_in(
                    &openhuman_core::core::runtime::DispatchCtx::ambient()
                )
                .len(),
                2,
                "scheduled turns lost the agent's post-turn hooks"
            );
        })
        .await;
    let (first, second) = tokio::join!(
        a.turn("a-first")
            .tool_hook(turn_hook.clone())
            .post_turn_hook(turn_hook)
            .send(),
        b.turn("b-first").send(),
    );
    let first = first.expect("first a");
    second.expect("first b");
    a.turn("a-second")
        .session(first.session_id)
        .send()
        .await
        .expect("resumed a");
    let mut session = scheduled
        .session_host(&scheduled.config, Some("scheduled-a"))
        .unwrap();
    scheduled
        .scope(async { session.turn_with_origin("a-scheduled", None).await })
        .await
        .unwrap();
    runtime.remove_agent("worker-a").await.expect("remove a");
    let replacement = runtime
        .agent(spec("worker-a", &provider_a))
        .expect("reuse id");
    replacement
        .run("replacement")
        .await
        .expect("replacement turn");
    eventually("all background post-turn observers", || {
        let events = events.lock().unwrap();
        (events
            .iter()
            .filter(|(_, phase, _)| phase == "turn")
            .count()
            == 10)
            .then_some(())
    })
    .await;

    let events = events.lock().unwrap();
    let count = |label: &str, phase: &str, owner: &str| {
        events
            .iter()
            .filter(|(l, p, o)| l == label && p == phase && o == owner)
            .count()
    };
    for phase in ["before", "after"] {
        assert_eq!(count("global", phase, "a"), 4);
        assert_eq!(count("global", phase, "b"), 1);
        assert_eq!(count("agent-a", phase, "a"), 3);
        assert_eq!(count("agent-b", phase, "b"), 1);
        assert_eq!(count("turn-a", phase, "a"), 1);
        assert_eq!(count("agent-a", phase, "b"), 0);
        assert_eq!(count("agent-b", phase, "a"), 0);
        assert_eq!(count("turn-a", phase, "b"), 0);
    }
    for (label, message) in [
        ("global", "a-first"),
        ("global", "b-first"),
        ("global", "a-second"),
        ("global", "a-scheduled"),
        ("global", "replacement"),
        ("agent-a", "a-first"),
        ("agent-a", "a-second"),
        ("agent-a", "a-scheduled"),
        ("agent-b", "b-first"),
        ("turn-a", "a-first"),
    ] {
        assert_eq!(
            count(label, "turn", message),
            1,
            "{label} / {message}: {events:?}"
        );
    }
    // Agent/turn callbacks never enter the runtime's process-global registry.
    assert_eq!(openhuman_core::agent::hooks::embedder_tool_hooks().len(), 1);
    assert_eq!(
        openhuman_core::agent::hooks::embedder_post_turn_hooks().len(),
        1
    );
}
