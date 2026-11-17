//! Title: Runtime defaults and per-agent overrides
//! Summary: Runtime defaults and per-agent overrides.
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
        .model_defaults(openhuman_embed::ModelDefaults {
            temperature: Some(0.4),
            max_tokens: Some(256),
            ..Default::default()
        })
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::example_provider(&provider)?)
        .build()
        .await?;
    // ANCHOR: defaults_overrides
    let inherited = runtime.agent(AgentSpec::new("inherited"))?;
    let override_provider = support::provider("override reply").await;
    let overridden = runtime.agent(
        AgentSpec::new("overridden")
            .provider(support::route(&override_provider, "override-model"))
            .access(openhuman_embed::Access::readonly())
            .model_defaults(openhuman_embed::ModelDefaults {
                temperature: Some(0.8),
                max_tokens: Some(128),
                ..Default::default()
            }),
    )?;
    let inherited_reply = inherited.run("Hello default").await?;
    let overridden_reply = overridden.run("Hello override").await?;
    if support::offline() {
        assert_eq!(inherited_reply.reply, "hello from the stub");
        let requests = support::chat_requests(&provider).await;
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
        assert_eq!(body["model"], "fixture");
        assert_eq!(body["temperature"], 0.4);
        assert_eq!(body["max_tokens"], 256);
    }
    assert_eq!(overridden_reply.reply, "override reply");
    let requests = support::chat_requests(&override_provider).await;
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body)?;
    assert_eq!(body["model"], "override-model");
    assert_eq!(body["temperature"], 0.8);
    assert_eq!(body["max_tokens"], 128);
    assert_eq!(runtime.defaults().model.temperature, Some(0.4));
    println!("default and override routes answered independently");
    // ANCHOR_END: defaults_overrides
    support::passed("defaults_overrides");
    Ok(())
}
