//! Title: Access tiers and explicit sandbox choices
//! Summary: Access tiers and explicit sandbox choices.
//! Run: offline with loopback stubs; no live path.
//! Feature: default

mod support;
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, Runtime, Workspace};

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
    // ANCHOR: sandbox_access
    let scratch = tempfile::tempdir()?;
    let marker = scratch.path().join("attempted-write");
    let attempted_write = support::tool_call_completion(
        "write_file",
        &serde_json::json!({
            "path": marker.display().to_string(), "content": "example"
        })
        .to_string(),
    );
    let read_provider = support::scripted_provider(vec![attempted_write], "write refused").await;
    let read = runtime.agent(
        AgentSpec::new("read")
            .provider(support::route(&read_provider, "fixture"))
            .action_dir(scratch.path())
            .access(openhuman_embed::Access::readonly())
            .definition(
                AgentDefinitionSpec::new().sandbox(openhuman_embed::SandboxModeSpec::ReadOnly),
            ),
    )?;
    let full = runtime.agent(AgentSpec::new("full").access(openhuman_embed::Access::full()))?;
    assert_ne!(read.config().autonomy.level, full.config().autonomy.level);
    assert!(!read.run("Write a marker").await?.reply.is_empty());
    assert!(
        !marker.exists(),
        "readonly sandbox must refuse the attempted write"
    );
    let requests = support::chat_requests(&read_provider).await;
    assert!(!support::tool_names(&requests[0]).contains(&"write_file".to_string()));
    println!("readonly sandbox refused a model-requested write");
    // ANCHOR_END: sandbox_access
    support::passed("sandbox_access");
    Ok(())
}
