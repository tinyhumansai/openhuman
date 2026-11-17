//! Title: A per-agent post-turn hook observes completed turns
//! Summary: A per-agent post-turn hook observes completed turns.
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
    // ANCHOR: hooks
    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let agent = runtime.agent(
        AgentSpec::new("hooked").post_turn_hook(std::sync::Arc::new(Counter(count.clone()))),
    )?;
    let sibling = runtime.agent(AgentSpec::new("sibling"))?;
    assert!(!agent.run("Hello hook").await?.reply.is_empty());
    support::eventually("post-turn hook", || {
        (count.load(std::sync::atomic::Ordering::SeqCst) == 1).then_some(())
    })
    .await;
    assert!(!sibling.run("Hello sibling").await?.reply.is_empty());
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 1);
    println!("post-turn hook isolated to its agent");
    // ANCHOR_END: hooks
    support::passed("hooks");
    Ok(())
}

struct Counter(std::sync::Arc<std::sync::atomic::AtomicUsize>);
#[async_trait::async_trait]
impl openhuman_embed::seams::LearningSink for Counter {
    async fn on_turn_complete(
        &self,
        _: &openhuman_embed::seams::TurnSummary,
    ) -> tinyagents_harness::error::Result<()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}
