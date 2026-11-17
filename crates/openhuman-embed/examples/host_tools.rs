//! Title: Host tools and HostOnly keep the advertised tool catalog exact
//! Summary: Host tools and HostOnly keep the advertised tool catalog exact.
//! Run: offline with loopback stubs; no live path.
//! Feature: default

mod support;
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, Runtime, ToolScopeSpec, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::route(&provider, "fixture"))
        .build()
        .await?;
    // ANCHOR: host_tools
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let recorded_calls = calls.clone();
    let tool_provider = support::scripted_provider(
        vec![support::tool_call_completion("host_ping", "{}")],
        "pong complete",
    )
    .await;
    let agent = runtime.agent(
        AgentSpec::new("host-tools")
            .provider(support::route(&tool_provider, "fixture"))
            .definition(
                AgentDefinitionSpec::new()
                    .bare_prompt("Use host_ping")
                    .tools(ToolScopeSpec::HostOnly),
            )
            .tools(move |_| {
                let recorded_calls = recorded_calls.clone();
                Box::pin(async move {
                    openhuman_embed::HostTurnTools::advertised(vec![Box::new(Ping(recorded_calls))])
                })
            }),
    )?;
    assert_eq!(agent.run("Ping").await?.reply, "pong complete");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    let requests = support::chat_requests(&tool_provider).await;
    assert_eq!(support::tool_names(&requests[0]), vec!["host_ping"]);
    assert!(support::tool_results(&requests[1]).contains("pong"));
    println!("host_ping executed once; only host tools advertised");
    // ANCHOR_END: host_tools
    support::passed("host_tools");
    Ok(())
}

struct Ping(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl openhuman_embed::Tool for Ping {
    fn name(&self) -> &str {
        "host_ping"
    }
    fn description(&self) -> &str {
        "Read a host supplied value"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({"type":"object","properties":{}})
    }
    fn policy(&self) -> openhuman_embed::ToolPolicy {
        openhuman_embed::ToolPolicy::read_only()
    }
    async fn execute(&self, _: serde_json::Value) -> anyhow::Result<openhuman_embed::ToolResult> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(openhuman_embed::ToolResult::success("pong"))
    }
}
