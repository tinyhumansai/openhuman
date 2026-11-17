//! Title: Lean runtime without background services
//! Summary: Lean runtime without background services.
//! Run: offline with loopback stubs; optional live via OPENHUMAN_EXAMPLE_LIVE.
//! Feature: default

mod support;
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, ToolScopeSpec, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    let runtime = openhuman_embed::RuntimeBuilder::lean()
        .config(support::offline_config())
        .services(openhuman_embed::ServiceSet::none())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::example_provider(&provider)?)
        .build()
        .await?;
    // ANCHOR: lean_headless
    let info = runtime.capabilities();
    assert_eq!(info.weight, openhuman_embed::WeightClass::Lean);
    assert!(info.services.is_empty());
    let agent = runtime.agent(
        AgentSpec::new("lean").definition(
            AgentDefinitionSpec::new()
                .bare_prompt("Reply briefly.")
                .tools(ToolScopeSpec::HostOnly),
        ),
    )?;
    assert!(!agent.run("Hello lean runtime").await?.reply.is_empty());
    println!("lean runtime runs a turn without background services");
    // ANCHOR_END: lean_headless
    support::passed("lean_headless");
    Ok(())
}
