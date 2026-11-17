//! Title: Structured output parsed and checked against the request
//! Summary: Structured output parsed and checked against the request.
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
    // ANCHOR: structured_output
    let json_provider = support::provider(r#"{"summary":"complete"}"#).await;
    let agent = runtime.agent(
        AgentSpec::new("structured")
            .provider(support::route(&json_provider, "fixture"))
            .definition(
                AgentDefinitionSpec::new()
                    .bare_prompt("Reply briefly.")
                    .tools(ToolScopeSpec::HostOnly),
            ),
    )?;
    let outcome = agent.turn("Analyze").response_format(openhuman_embed::complete::ResponseFormat::JsonSchema {
        name: "analysis".into(),
        schema: serde_json::json!({"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"]}),
    }).max_tokens(128).send().await?;
    assert_eq!(
        outcome.structured,
        Some(serde_json::json!({"summary":"complete"}))
    );
    let requests = support::chat_requests(&json_provider).await;
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
    assert_eq!(body["response_format"]["type"], "json_schema");
    assert_eq!(body["max_tokens"], 128);
    println!("validated summary: complete");
    // ANCHOR_END: structured_output
    support::passed("structured_output");
    Ok(())
}
