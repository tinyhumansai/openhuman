//! Title: Two agents with independent prompts and working folders
//! Summary: Two agents with independent prompts and working folders.
//! Run: offline; optional live via OPENHUMAN_EXAMPLE_LIVE=1 and BASE_URL/API_KEY/MODEL.
//! Feature: default

mod support;
use openhuman_embed::{AgentSpec, Runtime, Workspace};

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
    // ANCHOR: two_agents
    let analyst_dir = tempfile::tempdir()?;
    let writer_dir = tempfile::tempdir()?;
    let analyst = runtime.agent(
        AgentSpec::new("analyst")
            .system_prompt("ANALYST_PROMPT: summarize documents")
            .action_dir(analyst_dir.path())
            .access(openhuman_embed::Access::readonly()),
    )?;
    let writer = runtime.agent(
        AgentSpec::new("writer")
            .system_prompt("WRITER_PROMPT: compose explanations")
            .action_dir(writer_dir.path())
            .access(openhuman_embed::Access::full()),
    )?;
    assert_ne!(analyst.action_dir(), writer.action_dir());
    assert_ne!(analyst.home_dir(), writer.home_dir());
    assert_ne!(analyst.workspace_dir(), analyst.action_dir());
    assert!(!analyst.run("Analyze").await?.reply.is_empty());
    assert!(!writer.run("Explain").await?.reply.is_empty());
    if support::offline() {
        let requests = support::chat_requests(&provider).await;
        assert_eq!(requests.len(), 2);
        assert!(String::from_utf8_lossy(&requests[0].body).contains("ANALYST_PROMPT"));
        assert!(!String::from_utf8_lossy(&requests[0].body).contains("WRITER_PROMPT"));
        assert!(String::from_utf8_lossy(&requests[1].body).contains("WRITER_PROMPT"));
    }
    println!("two distinct prompts and action workspaces verified");
    // ANCHOR_END: two_agents
    support::passed("two_agents");
    Ok(())
}
