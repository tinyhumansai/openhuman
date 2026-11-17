//! Title: Streaming progress with a draining receiver
//! Summary: Streaming progress with a draining receiver.
//! Run: offline; optional live via OPENHUMAN_EXAMPLE_LIVE=1 and BASE_URL/API_KEY/MODEL.
//! Feature: default

mod support;
use openhuman_embed::{AgentSpec, Runtime, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::streaming_provider("hello from the stub").await;
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::example_provider(&provider)?)
        .build()
        .await?;
    // ANCHOR: streaming
    let agent = runtime.agent(AgentSpec::new("streamer"))?;
    let mut stream = agent.stream("Hello streaming");
    let mut count = 0;
    let mut deltas = String::new();
    let mut finished = None;
    while let Some(event) = stream.recv().await {
        match event {
            openhuman_embed::StreamEvent::Progress(progress) => {
                println!("{progress:?}");
                if let openhuman_embed::AgentProgress::TextDelta { delta, .. } = progress {
                    assert!(finished.is_none(), "text deltas precede Finished");
                    deltas.push_str(&delta);
                }
                count += 1;
            }
            openhuman_embed::StreamEvent::Finished(result) => finished = Some(result?),
        }
    }
    let outcome = finished.expect("stream ends with an outcome");
    assert!(!outcome.reply.is_empty());
    assert!(
        !deltas.is_empty(),
        "stream must publish visible text before Finished"
    );
    assert_eq!(deltas, outcome.reply);
    if support::offline() {
        let regular = agent.run("Hello streaming").await?;
        assert_eq!(
            outcome.reply, regular.reply,
            "streaming and collected turns agree"
        );
    }
    println!("progress events: {count}");
    // ANCHOR_END: streaming
    support::passed("streaming");
    Ok(())
}
