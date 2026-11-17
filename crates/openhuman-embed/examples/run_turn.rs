//! Title: Hello agent: a prompt in and a reply out
//! Summary: Hello agent: a prompt in and a reply out.
//! Run: offline; optional live via OPENHUMAN_EXAMPLE_LIVE=1 and BASE_URL/API_KEY/MODEL.
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
        .provider(support::example_provider(&provider)?)
        .build()
        .await?;
    // ANCHOR: run_turn
    let agent = runtime.agent(
        AgentSpec::new("hello").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    let outcome = agent.run("Say hello").await?;
    assert!(!outcome.reply.is_empty());
    if support::offline() {
        assert_eq!(outcome.reply, "hello from the stub");
    }
    println!("{}", outcome.reply);
    // ANCHOR_END: run_turn
    support::passed("run_turn");
    Ok(())
}
