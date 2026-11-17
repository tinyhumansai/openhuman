//! Title: Native custom providers, fallback and workload role pins
//! Summary: Inject deterministic native models and prove fallback and role selection without inference HTTP.
//! Run: offline; no live inference path.
//! Feature: default

mod support;
use openhuman_embed::providers::{ChatModel, ModelRequest, ModelResponse};
use openhuman_embed::{AgentDefinitionSpec, AgentSpec, Provider, Runtime, ToolScopeSpec};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let failed = Arc::new(AtomicUsize::new(0));
    let fallback = Arc::new(AtomicUsize::new(0));
    let pinned = Arc::new(AtomicUsize::new(0));
    // ANCHOR: custom_provider
    let provider = Provider::custom_with_fallback(
        Arc::new(LocalModel {
            reply: None,
            calls: failed.clone(),
        }),
        vec![Arc::new(LocalModel {
            reply: Some("fallback answer"),
            calls: fallback.clone(),
        })],
    )
    .model("local-custom")
    .role(
        "coding",
        Arc::new(LocalModel {
            reply: Some("coding answer"),
            calls: pinned.clone(),
        }),
    );
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .backend_url(backend.uri())
        .provider(provider)
        .build()
        .await?;
    let definition = || {
        AgentDefinitionSpec::new()
            .bare_prompt("Reply briefly")
            .tools(ToolScopeSpec::HostOnly)
    };
    let regular = runtime.agent(AgentSpec::new("fallback-agent").definition(definition()))?;
    assert_eq!(regular.run("Hello").await?.reply, "fallback answer");
    assert_eq!(failed.load(Ordering::SeqCst), 1);
    assert_eq!(fallback.load(Ordering::SeqCst), 1);
    let coding = runtime.agent(
        AgentSpec::new("coding-agent")
            .model("hint:coding")
            .definition(definition()),
    )?;
    assert_eq!(
        coding.run("Explain a function").await?.reply,
        "coding answer"
    );
    assert_eq!(pinned.load(Ordering::SeqCst), 1);
    assert_eq!(
        failed.load(Ordering::SeqCst),
        1,
        "coding bypasses the primary chain"
    );
    // ANCHOR_END: custom_provider
    println!("native fallback and coding role pin verified");
    support::passed("custom_provider");
    Ok(())
}

struct LocalModel {
    reply: Option<&'static str>,
    calls: Arc<AtomicUsize>,
}
#[async_trait::async_trait]
impl ChatModel<()> for LocalModel {
    async fn invoke(
        &self,
        _: &(),
        _: ModelRequest,
    ) -> openhuman_embed::providers::Result<ModelResponse> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.reply {
            Some(reply) => Ok(ModelResponse::assistant(reply)),
            None => Err(openhuman_embed::providers::Error::Model(
                "local primary unavailable".into(),
            )),
        }
    }
}
