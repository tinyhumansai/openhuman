//! Title: Runtime lifecycle events and live hook registration
//! Summary: Observe ordered metadata and add/remove a runtime-wide hook while agents keep running.
//! Run: offline with a loopback provider; no live path.
//! Feature: default

mod support;
use openhuman_embed::{AgentSpec, Runtime, RuntimeEventKind};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("event reply").await;
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .backend_url(backend.uri())
        .provider(support::route(&provider, "fixture"))
        .build()
        .await?;
    // ANCHOR: runtime_events
    let mut events = runtime.events();
    let count = Arc::new(AtomicUsize::new(0));
    runtime.post_turn_hook("live-counter", Some(Arc::new(Counter(count.clone()))));
    let agent = runtime.agent(AgentSpec::new("observed"))?;
    assert_eq!(agent.run("private payload one").await?.reply, "event reply");
    support::eventually("runtime hook", || {
        (count.load(Ordering::SeqCst) == 1).then_some(())
    })
    .await;
    runtime.post_turn_hook("live-counter", None);
    assert_eq!(agent.run("private payload two").await?.reply, "event reply");
    assert_eq!(
        count.load(Ordering::SeqCst),
        1,
        "removed hook stops observing turns"
    );
    runtime.remove_agent(agent.id()).await?;
    let mut previous = 0;
    let (mut added, mut started, mut ended) = (0, 0, 0);
    loop {
        let event =
            tokio::time::timeout(std::time::Duration::from_secs(5), events.recv()).await??;
        assert!(event.sequence > previous);
        previous = event.sequence;
        assert_eq!(event.agent_id.as_deref(), Some("observed"));
        assert!(!serde_json::to_string(&event)?.contains("private payload"));
        match event.kind {
            RuntimeEventKind::AgentAdded => added += 1,
            RuntimeEventKind::TurnStarted => {
                assert!(event.turn_id.is_some());
                started += 1;
            }
            RuntimeEventKind::TurnEnded { success } => {
                assert!(success);
                ended += 1;
            }
            RuntimeEventKind::AgentRemoved => break,
            _ => {}
        }
    }
    assert_eq!((added, started, ended), (1, 2, 2));
    // ANCHOR_END: runtime_events
    println!("ordered content-free lifecycle events and live hook removal verified");
    support::passed("runtime_events");
    Ok(())
}
struct Counter(Arc<AtomicUsize>);
#[async_trait::async_trait]
impl openhuman_embed::seams::LearningSink for Counter {
    async fn on_turn_complete(
        &self,
        _: &openhuman_embed::seams::TurnSummary,
    ) -> tinyagents_harness::error::Result<()> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
